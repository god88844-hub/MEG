import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash, randomUUID } from "node:crypto";
import { McpHost, type McpConfig } from "./mcp-host";

export interface PromptResource { name:string; description:string; body:string; source:string }
export interface Plugin { name:string; path:string; enabled:boolean }
export function configRoot(){return process.env.MEG_CONFIG_DIR??(process.env.APPDATA?path.join(process.env.APPDATA,"MEG"):path.join(os.homedir(),".config","meg"));}
function json(file:string,fallback:any={}){if(!fs.existsSync(file))return fallback;if(fs.statSync(file).size>1024*1024)throw new Error(`Configuration exceeds 1 MiB: ${path.basename(file)}`);return JSON.parse(fs.readFileSync(file,"utf8"));}
function atomic(file:string,value:unknown){fs.mkdirSync(path.dirname(file),{recursive:true});const tmp=file+`.${randomUUID()}.tmp`;try{fs.writeFileSync(tmp,JSON.stringify(value,null,2),{flag:"wx"});fs.renameSync(tmp,file);}finally{if(fs.existsSync(tmp))fs.unlinkSync(tmp);}}
function inside(root:string,file:string){const relative=path.relative(fs.realpathSync(root),fs.realpathSync(file));if(relative===".."||relative.startsWith(".."+path.sep)||path.isAbsolute(relative))throw new Error("Plugin resource escapes its directory");return file;}
function resource(file:string,name?:string):PromptResource{
  if(fs.statSync(file).size>64*1024)throw new Error(`Prompt resource exceeds 64 KiB: ${path.basename(file)}`);
  const text=fs.readFileSync(file,"utf8");const front=text.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/);const field=(key:string)=>front?.[1].match(new RegExp(`^${key}:\\s*(.+)$`,"m"))?.[1].trim().replace(/^['"]|['"]$/g,"");
  const id=name??field("name")??path.basename(path.dirname(file));if(!/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$/.test(id))throw new Error(`Invalid prompt resource name: ${id}`);
  return {name:id,description:field("description")??id,body:front?text.slice(front[0].length):text,source:file};
}
function skills(directory:string):PromptResource[]{if(!fs.existsSync(directory))return [];const result:PromptResource[]=[];for(const entry of fs.readdirSync(directory,{withFileTypes:true}).slice(0,128)){const file=path.join(directory,entry.name,"SKILL.md");if(entry.isDirectory()&&fs.existsSync(file))result.push(resource(inside(directory,file)));}return result;}
export class WorkspaceExtensions{
  readonly workspace:string;readonly host=new McpHost();private registryFile:string;
  private pluginServers=new Map<string,Set<string>>();
  constructor(workspace:string){this.workspace=fs.realpathSync(workspace);this.registryFile=path.join(this.workspace,".meg","plugins.json");}
  private plugins():Plugin[]{const value=json(this.registryFile,[]);if(!Array.isArray(value)||value.length>64)throw new Error("Invalid plugin registry");return value;}
  private manifest(directory:string){const candidates=["meg-plugin.json",path.join(".claude-plugin","plugin.json"),"plugin.json"];const file=candidates.map(p=>path.join(directory,p)).find(p=>fs.existsSync(p));if(!file)throw new Error("Plugin needs meg-plugin.json or .claude-plugin/plugin.json");return json(inside(directory,file));}
  private configs():Record<string,McpConfig>{
    const result:Record<string,McpConfig>={};
    for(const file of [path.join(configRoot(),"mcp.json"),path.join(this.workspace,".mcp.json"),path.join(this.workspace,"mcp_config.json"),path.join(this.workspace,".meg","mcp.json")])Object.assign(result,json(file).mcpServers??{});
    for(const plugin of this.plugins().filter(p=>p.enabled)){let manifest;try{manifest=this.manifest(plugin.path);}catch{continue;}const owned=new Set<string>();for(const[name,config]of Object.entries(manifest.mcpServers??{})){const server=`${plugin.name}_${name}`;if(result[server])throw new Error(`Duplicate MCP server name: ${server}`);result[server]=config as McpConfig;owned.add(server);}this.pluginServers.set(plugin.name,owned);}
    for(const name of Object.keys(result))if(!/^[a-zA-Z0-9_-]{1,80}$/.test(name))throw new Error("MCP server names must use letters, numbers, '-' or '_'");
    return result;
  }
  private fingerprint(name:string,config:McpConfig){return createHash("sha256").update(JSON.stringify([this.workspace,name,config])).digest("hex");}
  private trust(name:string,config:McpConfig,enabled:boolean){const file=path.join(configRoot(),"mcp-trust.json");const value=json(file);const key=createHash("sha256").update(this.workspace+"\0"+name).digest("hex");if(enabled)value[key]=this.fingerprint(name,config);else delete value[key];atomic(file,value);}
  async reconnectTrusted(){const trusted=json(path.join(configRoot(),"mcp-trust.json"));for(const[name,config]of Object.entries(this.configs())){const key=createHash("sha256").update(this.workspace+"\0"+name).digest("hex");if(!config.disabled&&trusted[key]===this.fingerprint(name,config)&&!this.host.status(name).connected){await this.host.connect(name,config,this.workspace).catch(()=>{});}}}
  async catalog(){
    const byName=new Map<string,PromptResource>();const commands:PromptResource[]=[];let used=0;
    const directories=[process.env.MEG_BUILTIN_SKILLS,path.join(configRoot(),"skills"),process.env.MEG_USER_SKILLS_DIR??path.join(os.homedir(),".agents","skills"),path.join(this.workspace,".claude","skills"),path.join(this.workspace,".agents","skills"),path.join(this.workspace,".meg","skills")].filter((p):p is string=>!!p);
    for(const directory of directories)for(const skill of skills(directory)){used+=skill.body.length;if(used>1024*1024)throw new Error("Skill catalog exceeds 1 MiB");byName.set(skill.name,skill);}
    const plugins=this.plugins().map(plugin=>{let error:string|undefined;try{const manifest=this.manifest(plugin.path);if(plugin.enabled){
      const folders=Array.isArray(manifest.skills)?manifest.skills:["skills"];
      for(const folder of folders){const full=path.join(plugin.path,folder);if(fs.existsSync(full))for(const skill of skills(inside(plugin.path,full)))byName.set(`${plugin.name}.${skill.name}`,{...skill,name:`${plugin.name}.${skill.name}`});}
      const commandDir=path.join(plugin.path,"commands");if(fs.existsSync(commandDir))for(const file of fs.readdirSync(commandDir).filter(f=>f.endsWith(".md")).slice(0,64))commands.push(resource(inside(plugin.path,path.join(commandDir,file)),`${plugin.name}.${path.basename(file,".md")}`));
    }}catch(e){error=e instanceof Error?e.message:"Plugin could not be loaded";}return {...plugin,error};});
    if([...byName.values(),...commands].reduce((sum,item)=>sum+Buffer.byteLength(item.body),0)>1024*1024)throw new Error("Combined skill and plugin prompt catalog exceeds 1 MiB");
    const configs=this.configs();return {skills:[...byName.values()],commands,plugins,servers:Object.entries(configs).map(([name,config])=>({name,transport:config.url?(config.type??"http"):"stdio",disabled:!!config.disabled,...this.host.status(name)})),tools:this.host.allTools()};
  }
  async pluginCommand(command:string){const match=command.trim().match(/^(\S+)(?:\s+([\s\S]*))?$/);const action=match?.[1]==="install"?"add":match?.[1];const argument=(match?.[2]??"").trim().replace(/^"|"$/g,"");let list=this.plugins();
    if(!action||action==="list")return {catalog:await this.catalog(),message:list.length?list.map(p=>`${p.name}: ${p.enabled?"enabled":"disabled"} (${p.path})`).join("\n"):"No plugins installed. /plugins add <local-plugin-directory>"};
    if(action==="add"){if(!argument)throw new Error("Usage: /plugins add <local directory>");const directory=fs.realpathSync(path.resolve(this.workspace,argument));const manifest=this.manifest(directory);const name=manifest.name??path.basename(directory);if(!/^[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}$/.test(name))throw new Error("Invalid plugin name");if(list.some(p=>p.name===name))throw new Error("Plugin name already exists; remove it before replacing it");list.push({name,path:directory,enabled:true});}
    else if(["enable","disable","remove"].includes(action!)){const plugin=list.find(p=>p.name===argument);if(!plugin)throw new Error("Unknown plugin name");if(action==="enable")this.manifest(plugin.path);for(const server of this.pluginServers.get(plugin.name)??[])await this.host.stopServer(server);this.pluginServers.delete(plugin.name);if(action==="remove")list=list.filter(p=>p!==plugin);else plugin.enabled=action==="enable";}
    else throw new Error("Usage: /plugins [list|add <directory>|enable <name>|disable <name>|remove <name>]");
    atomic(this.registryFile,list);return {catalog:await this.catalog(),message:`Plugin ${action}: ${argument}`};
  }
  async mcpCommand(command:string){const[action,...rest]=command.trim().split(/\s+/);const name=rest[0];
    if(!action||action==="list"||action==="tools")return {catalog:await this.catalog(),message:action==="tools"?this.host.allTools().map(t=>`${t.alias}: ${t.description??t.name}`).join("\n")||"No MCP tools connected. /mcp start <server>":(await this.catalog()).servers.map(s=>`${s.name}: ${s.disabled?"disabled":s.connected?"connected":s.connecting?"connecting":"stopped"}${s.error?" · "+s.error:""}`).join("\n")||"No MCP servers configured. Add mcpServers to .meg/mcp.json."};
    const configs=this.configs();const names=name==="all"?Object.keys(configs):[name];if(!name||names.some(n=>!configs[n]))throw new Error("Unknown MCP server. /mcp lists configured names.");
    if(action==="start"||action==="restart"){for(const n of names){if(action==="restart")await this.host.stopServer(n);await this.host.connect(n,configs[n],this.workspace);this.trust(n,configs[n],true);}}
    else if(action==="stop"){for(const n of names){this.trust(n,configs[n],false);await this.host.stopServer(n);}}
    else throw new Error("Usage: /mcp [list|tools|start <server|all>|stop <server|all>|restart <server>]");
    return {catalog:await this.catalog(),message:`MCP ${action}: ${names.join(", ")}`};
  }
  async tool(alias:string,args:Record<string,unknown>,readOnly:boolean,signal?:AbortSignal){const tool=this.host.allTools().find(t=>t.alias===alias);if(!tool)throw new Error("MCP tool is not connected; use /mcp start <server>");if(readOnly&&!tool.readOnly)throw new Error("MCP tool is disabled in Plan mode: no read-only annotation");const result:any=await this.host.callTool(tool.server,tool.name,args,signal);if(result?.isError)throw new Error((result.content??[]).filter((p:any)=>p.type==="text").map((p:any)=>p.text).join("\n")||"MCP tool failed");return result;}
  async readSkill(name:string){const skill=(await this.catalog()).skills.find(s=>s.name===name);if(!skill)throw new Error("Unknown skill");return skill;}
}
