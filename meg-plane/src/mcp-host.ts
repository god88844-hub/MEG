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
        for(const tool of page.tools)tools.push({server:name,name:tool.name,alias:`mcp:${name}:${tool.name}`,description:tool.description,inputSchema:tool.inputSchema,readOnly:tool.annotations?.readOnlyHint===true});
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
