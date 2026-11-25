export function sanitizeMcpAlias(server: string, tool: string): string {
  const clean = (s: string) => s.replace(/[^a-zA-Z0-9_-]/g, "_").replace(/_+/g, "_").replace(/^_+|_+$/g, "") || "x";
  const raw = `mcp__${clean(server)}__${clean(tool)}`;
  if (raw.length <= 64) return raw;
  let h = 2166136261 >>> 0;
  for (let i = 0; i < raw.length; i++) {
    h = Math.imul(h ^ raw.charCodeAt(i), 16777619) >>> 0;
  }
  return `${raw.slice(0, 55)}_${h.toString(16).padStart(8, "0")}`;
}
// Layer 2 owns MCP transports and their complete lifecycle.
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport, getDefaultEnvironment } from "@modelcontextprotocol/sdk/client/stdio.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import type { Transport } from "@modelcontextprotocol/sdk/shared/transport.js";

export interface McpTool { server:string; name:string; alias:string; description?:string; inputSchema:Record<string,unknown>; readOnly:boolean }
export interface McpConfig { command?:string; args?:string[]; env?:Record<string,string>; url?:string; type?:string; headers?:Record<string,string>; disabled?:boolean }
type Entry={client:Client; transport:Transport; tools:McpTool[]; abort:AbortController};
export class McpHost {
  private servers=new Map<string,Entry>();
  private starting=new Map<string,Promise<McpTool[]>>();
  private errors=new Map<string,string>();
  async startServer(name:string,command:string,args:string[]=[]):Promise<McpTool[]>{return this.connect(name,{command,args});}
  async connect(name:string,config:McpConfig,cwd?:string):Promise<McpTool[]>{
    if(this.starting.has(name))return this.starting.get(name)!;
    if(this.servers.has(name))return this.servers.get(name)!.tools;
    if(config.disabled)throw new Error(`MCP server '${name}' is disabled in configuration`);
    const policy=routeCliOrMcp(name);
    if(policy.route==="cli")throw new Error(`CLI_OVER_MCP_BLOCKED: ${policy.reason}`);
    if(policy.route==="denied"&&process.env.MEG_STRICT_MCP_ALLOWLIST==="1")throw new Error(`MCP_ALLOWLIST_BLOCKED: ${policy.reason}`);
    const operation=this.open(name,config,cwd);this.starting.set(name,operation);
    try{return await operation;}finally{this.starting.delete(name);}
  }
  private async open(name:string,config:McpConfig,cwd?:string):Promise<McpTool[]>{
    const abort=new AbortController();const client=new Client({name:"meg",version:"0.1.0"},{capabilities:{}});let transport:Transport;
    if(config.url){
      const url=new URL(config.url);if(!["http:","https:"].includes(url.protocol)||url.username||url.password)throw new Error("MCP URL must be an HTTP(S) URL without embedded credentials");
      const options={requestInit:{headers:expandEnvironment(config.headers??{})}};
      transport=config.type==="sse"?new SSEClientTransport(url,options):new StreamableHTTPClientTransport(url,options);
    }else{
      if(typeof config.command!=="string"||!config.command)throw new Error(`MCP '${name}' needs a command or URL`);
      if(config.args&&!config.args.every(a=>typeof a==="string"))throw new Error(`MCP '${name}' has invalid arguments`);
      const stdio=new StdioClientTransport({command:config.command,args:config.args??[],cwd,env:{...getDefaultEnvironment(),...expandEnvironment(config.env??{})},stderr:"pipe"});
      stdio.stderr?.on("data",()=>{});transport=stdio;
    }
    const entry:Entry={client,transport,tools:[],abort};this.servers.set(name,entry);
    client.onclose=()=>{if(this.servers.get(name)===entry)this.servers.delete(name);};
    try{
      await client.connect(transport,{timeout:15_000,signal:abort.signal});
      let cursor:string|undefined;const tools:McpTool[]=[];
      do{const page=await client.listTools(cursor?{cursor}:undefined,{timeout:15_000,signal:abort.signal});
        for(const tool of page.tools)tools.push({server:name,name:tool.name,alias:sanitizeMcpAlias(name,tool.name),description:tool.description,inputSchema:tool.inputSchema,readOnly:tool.annotations?.readOnlyHint===true});
        if(tools.length>500)throw new Error("MCP server advertises too many tools");cursor=page.nextCursor;
      }while(cursor);
      entry.tools=tools;this.errors.delete(name);return tools;
    }catch{
      this.servers.delete(name);abort.abort();await client.close().catch(()=>{});await transport.close().catch(()=>{});
      const message=`Could not initialize MCP server '${name}'; check its command, arguments and environment`;
      this.errors.set(name,message);throw new Error(message);
    }
  }
  async callTool(server:string,tool:string,args:Record<string,unknown>,signal?:AbortSignal):Promise<unknown>{
    const entry=this.servers.get(server);if(!entry||!entry.tools.some(t=>t.name===tool))throw new Error(`MCP tool is not connected: ${server}/${tool}`);
    return entry.client.callTool({name:tool,arguments:args},undefined,{timeout:60_000,signal});
  }
  allTools():McpTool[]{return [...this.servers.values()].flatMap(s=>s.tools);}
  status(name:string){return {connected:this.servers.has(name)&&!this.starting.has(name),connecting:this.starting.has(name),error:this.errors.get(name)};}
  async stopServer(name:string){const entry=this.servers.get(name);if(!entry)return;this.servers.delete(name);entry.abort.abort();await entry.client.close().catch(()=>{});await entry.transport.close().catch(()=>{});}
  async stopAll(){await Promise.all([...this.servers.keys()].map(name=>this.stopServer(name)));}
}
function expandEnvironment(values:Record<string,string>):Record<string,string>{const result:Record<string,string>={};for(const[key,value]of Object.entries(values)){if(typeof value!=="string")throw new Error("MCP environment/header values must be strings");result[key]=value.replace(/\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g,(_,name)=>{if(process.env[name]===undefined)throw new Error(`Missing MCP environment variable: ${name}`);return process.env[name]!;});}return result;}

// ===========================================================================
// P3-06: CLI > MCP Enforcer, Hardened Lifecycle, Verdict/Transport Split,
// Ephemeral uvx Spawner, Scoped Action Tokens & Parallel Web-Research Connector
// ===========================================================================

import { createHmac, randomBytes } from "node:crypto";

export const CLI_FIRST_BINARIES = [
  "gh",
  "supabase",
  "stripe",
  "vercel",
  "git",
  "curl",
] as const;

export const DEFAULT_ALLOWED_MCP_SERVERS = [
  "chrome-devtools",
  "Context7",
  "sentry",
] as const;

export type CliOrMcpRoute =
  | { route: "cli"; binary: string; reason: string }
  | { route: "mcp"; server: string; reason: string }
  | { route: "denied"; target: string; reason: string };

/**
 * CLI > MCP Enforcer:
 * - Routes `gh`, `supabase`, `stripe`, `vercel`, `git`, `curl` strictly to native CLIs.
 * - Restricts default MCP servers to `chrome-devtools`, `Context7`, and `sentry`
 *   (unless explicitly opted in via `extraAllowedMcp`).
 */
export function routeCliOrMcp(
  target: string,
  options: { extraAllowedMcp?: readonly string[] } = {},
): CliOrMcpRoute {
  const clean = target.trim();
  const low = clean.toLowerCase();
  const cliMatch = CLI_FIRST_BINARIES.find(
    (b) => low === b || low === `${b}-mcp` || low.startsWith(`${b}_`) || low.startsWith(`${b}-`),
  );
  if (cliMatch) {
    return {
      route: "cli",
      binary: cliMatch,
      reason: `CLI > MCP policy: use native '${cliMatch}' CLI directly instead of MCP wrapper`,
    };
  }
  const allowed = new Set(
    [...DEFAULT_ALLOWED_MCP_SERVERS, ...(options.extraAllowedMcp ?? [])].map((s) => s.toLowerCase()),
  );
  if (allowed.has(low)) {
    return {
      route: "mcp",
      server: clean,
      reason: `Allowed default MCP server '${clean}'`,
    };
  }
  return {
    route: "denied",
    target: clean,
    reason: `MCP server '${clean}' blocked by default allowlist (${DEFAULT_ALLOWED_MCP_SERVERS.join(", ")})`,
  };
}

/** Verdict-vs-transport error split so harness retries transport faults without blaming tool logic. */
export class McpTransportError extends Error {
  readonly kind = "transport" as const;
  constructor(message: string) {
    super(message);
    this.name = "McpTransportError";
  }
}

export class McpVerdictError extends Error {
  readonly kind = "verdict" as const;
  constructor(message: string) {
    super(message);
    this.name = "McpVerdictError";
  }
}

export function classifyMcpError(err: unknown): McpTransportError | McpVerdictError {
  if (err instanceof McpTransportError || err instanceof McpVerdictError) return err;
  const msg = err instanceof Error ? err.message : String(err);
  if (/\b(ECONNREFUSED|EPIPE|ETIMEDOUT|timeout|timed out|abort|closed|Could not initialize)\b/i.test(msg)) {
    return new McpTransportError(msg);
  }
  return new McpVerdictError(msg);
}

/** Mint a short-lived scoped per-action token (zero raw user token passthrough). */
export function mintScopedActionToken(
  secretSeed: string,
  server: string,
  action: string,
  ttlSeconds = 60,
): { token: string; expiresAt: number; scope: string } {
  const expiresAt = Date.now() + ttlSeconds * 1000;
  const nonce = randomBytes(8).toString("hex");
  const scope = `${server}:${action}`;
  const sig = createHmac("sha256", secretSeed)
    .update(`${scope}|${expiresAt}|${nonce}`)
    .digest("hex")
    .slice(0, 32);
  return {
    token: `meg_act_${nonce}_${sig}`,
    expiresAt,
    scope,
  };
}

/**
 * Build an on-demand ephemeral `uvx` MCP server config with:
 * - Zero turn-context exposure (only explicit whitelisted env vars + scoped action token)
 * - No raw user credential passthrough.
 */
export function buildEphemeralUvxMcpConfig(
  packageName: string,
  args: readonly string[],
  scopedActionToken: string,
): McpConfig {
  if (!/^[a-zA-Z0-9._@-]+$/.test(packageName)) {
    throw new Error(`Invalid uvx package name: ${packageName}`);
  }
  return {
    command: "uvx",
    args: ["--from", packageName, packageName, ...args],
    env: {
      MEG_SCOPED_ACTION_TOKEN: scopedActionToken,
      MEG_ZERO_TURN_CONTEXT: "1",
    },
  };
}

/** Non-blocking async startup helper + per-caller timeout wrapper around McpHost. */
export function startMcpNonBlocking(
  host: McpHost,
  name: string,
  config: McpConfig,
  cwd?: string,
): { started: true; ready: Promise<McpTool[]> } {
  const ready = host.connect(name, config, cwd);
  return { started: true, ready };
}

export async function callMcpWithPerCallerTimeout<T>(
  operation: (signal: AbortSignal) => Promise<T>,
  timeoutMs: number,
): Promise<T> {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(new Error(`MCP caller timeout after ${timeoutMs}ms`)), timeoutMs);
  try {
    return await Promise.race([
      operation(ac.signal),
      new Promise<never>((_, reject) => {
        ac.signal.addEventListener(
          "abort",
          () => reject(new McpTransportError(`MCP caller timeout after ${timeoutMs}ms`)),
          { once: true },
        );
      }),
    ]);
  } catch (e) {
    throw classifyMcpError(e);
  } finally {
    clearTimeout(timer);
  }
}

export interface WebResearchHit {
  url: string;
  title: string;
  excerpt: string;
  ok: boolean;
  error?: string;
}

/** Parallel/browser web-research connector with concurrency bound and offline-injectable fetcher. */
export class ParallelWebResearchConnector {
  constructor(
    private fetcher: (url: string, signal: AbortSignal) => Promise<{ title: string; text: string }>,
    private timeoutMs = 8_000,
  ) {}

  async fetchParallel(urls: readonly string[]): Promise<WebResearchHit[]> {
    const tasks = urls.map(async (url): Promise<WebResearchHit> => {
      try {
        const res = await callMcpWithPerCallerTimeout(
          (sig) => this.fetcher(url, sig),
          this.timeoutMs,
        );
        return { url, title: res.title, excerpt: res.text.slice(0, 600), ok: true };
      } catch (e) {
        return {
          url,
          title: "",
          excerpt: "",
          ok: false,
          error: e instanceof Error ? e.message : String(e),
        };
      }
    });
    return Promise.all(tasks);
  }
}