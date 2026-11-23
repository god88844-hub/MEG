// JSON-RPC 2.0 over stdio — the control-plane seam for the native Rust TUI,
// workspace extensions, MCP servers and native worker delegation.

import { ModelRouter, type ModelEntry } from "./router";
import { SessionTree } from "./session-tree";
import { McpHost } from "./mcp-host";
import { WorkspaceExtensions } from "./extensions";
import { AgentManager } from "./agents";
import fs from "node:fs";

const router = new ModelRouter();
const sessions = new Map<string, SessionTree>();
const mcp = new McpHost();
const workspaces=new Map<string,WorkspaceExtensions>();
const delegates=new AgentManager(4);
const activeRequests=new Map<string|number,{abort:AbortController;workspace?:string;method:string}>();
type RpcContext={signal:AbortSignal;notify:(event:unknown)=>void};
async function workspaceRuntime(workspace:string){if(typeof workspace!=="string"||!fs.statSync(workspace).isDirectory())throw new Error("Workspace directory is unavailable");const key=fs.realpathSync(workspace);let runtime=workspaces.get(key);if(!runtime){runtime=new WorkspaceExtensions(key);workspaces.set(key,runtime);await runtime.reconnectTrusted();}return runtime;}

type Handler = (params: any) => Promise<unknown> | unknown;

const handlers: Record<string, Handler> = {
  "router.probeLocal": async (p: { baseUrl?: string }) => {
    const models: ModelEntry[] = await ModelRouter.probeLocal(p?.baseUrl);
    for (const m of models) router.register(m);
    for (const m of ModelRouter.cloudLadder()) router.register(m); // honest not-configured
    return router.fleet();
  },
  "router.route": (p: { goal: string; role?: string }) => router.route(p),
  "session.create": () => {
    const id = `s-${Date.now().toString(36)}`;
    sessions.set(id, new SessionTree());
    return { sessionId: id };
  },
  "session.append": (p: { sessionId: string; parentId: string | null; kind: string; content: string }) => {
    const tree = sessions.get(p.sessionId);
    if (!tree) throw new Error(`no session ${p.sessionId}`);
    return tree.append(p.parentId, p.kind as any, p.content);
  },
  "session.forkPath": (p: { sessionId: string; nodeId: string }) => {
    const tree = sessions.get(p.sessionId);
    if (!tree) throw new Error(`no session ${p.sessionId}`);
    return tree.forkFrom(p.nodeId);
  },
  "mcp.startServer": async (p: { name: string; command: string; args?: string[] }) =>
    mcp.startServer(p.name, p.command, p.args ?? []),
  "mcp.callTool": (p: { server: string; tool: string; args: Record<string, unknown> }) =>
    mcp.callTool(p.server, p.tool, p.args),
  "mcp.allTools": () => mcp.allTools(),
};

export async function handleRpc(method: string, params: any, context:RpcContext={signal:new AbortController().signal,notify:()=>{}}): Promise<unknown> {
  if(method==="runtime.ping")return {ready:true,version:"0.1.0"};
  if(method==="runtime.cancel"){const request=activeRequests.get(params.requestId);request?.abort.abort();if(request?.workspace&&request.method==="mcp.command"){const runtime=workspaces.get(fs.realpathSync(request.workspace));await runtime?.host.stopAll();}return {cancelled:true};}
  if(method==="agents.status")return {agents:delegates.status()};
  if(method==="agents.run")return delegates.run(params,context.signal,context.notify);
  if(method.startsWith("runtime.")||method==="skills.read"||method==="plugins.command"||method==="mcp.command"){
    const runtime=await workspaceRuntime(params.workspace);
    if(method==="runtime.catalog")return {catalog:await runtime.catalog(),message:"Skills, plugins and MCP catalog refreshed"};
    if(method==="skills.read")return runtime.readSkill(params.name);
    if(method==="plugins.command")return runtime.pluginCommand(params.command??"");
    if(method==="mcp.command")return runtime.mcpCommand(params.command??"");
    if(method==="runtime.tool")return runtime.tool(params.tool,params.args??{},!!params.readOnly,context.signal);
  }
  const handler = handlers[method];
  if (!handler) throw new Error(`unknown method: ${method}`);
  return handler(params);
}

// stdio JSON-RPC 2.0 dispatch loop
if (import.meta.main) {
  let buf = "";
  const respond=(value:unknown)=>{const line=JSON.stringify(value);if(Buffer.byteLength(line)>4*1024*1024)throw new Error("Control-plane result exceeds 4 MiB");process.stdout.write(line+"\n");};
  const dispatch=async(msg:any)=>{
    if(!msg||typeof msg!=="object"||typeof msg.method!=="string")return;
    const id=msg.id;const abort=new AbortController();if(id!==undefined)activeRequests.set(id,{abort,workspace:msg.params?.workspace,method:msg.method});
    try{const result=await handleRpc(msg.method,msg.params??{},{signal:abort.signal,notify:event=>respond({jsonrpc:"2.0",method:"agent.event",params:{...(event as object),requestId:id}})});if(id!==undefined)respond({jsonrpc:"2.0",id,result});}
    catch(error){if(id!==undefined)respond({jsonrpc:"2.0",id,error:{code:-32603,message:error instanceof Error?error.message:"Control-plane operation failed"}});}
    finally{if(id!==undefined)activeRequests.delete(id);}
  };
  process.stdin.setEncoding("utf-8");
  process.stdin.on("data", (chunk: string) => {
    buf += chunk;
    if(Buffer.byteLength(buf)>4*1024*1024){process.stderr.write("Control-plane input exceeded limit\n");process.exit(1);}
    let idx: number;
    while ((idx = buf.indexOf("\n")) !== -1) {
      const line = buf.slice(0, idx).trim();
      buf = buf.slice(idx + 1);
      if (!line) continue;
      let msg: any;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      if (!msg || typeof msg !== "object") {
        continue;
      }
      void dispatch(msg);
    }
  });
  const shutdown=async()=>{for(const request of activeRequests.values())request.abort.abort();delegates.cancelAll();await Promise.all([mcp.stopAll(),...[...workspaces.values()].map(r=>r.host.stopAll())]);};
  process.stdin.on("end",()=>void shutdown().finally(()=>process.exit(0)));
  process.on("SIGTERM",()=>void shutdown().finally(()=>process.exit(0)));
  process.stderr.write("meg-plane control plane ready (JSON-RPC 2.0 over stdio)\n");
}
