import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { ListToolsRequestSchema,CallToolRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import fs from "node:fs";
if(process.argv[2])fs.appendFileSync(process.argv[2],"started\n");
const server=new Server({name:"meg-offline-fixture",version:"1"},{capabilities:{tools:{}}});
server.setRequestHandler(ListToolsRequestSchema,async()=>({tools:[
  {name:"echo",description:"Offline echo fixture",inputSchema:{type:"object",properties:{text:{type:"string"}}},annotations:{readOnlyHint:true}},
  {name:"mutate",description:"Write-capable fixture",inputSchema:{type:"object"}},
  {name:"fail",description:"Returns an MCP tool error",inputSchema:{type:"object"}},
]}));
server.setRequestHandler(CallToolRequestSchema,async request=>{if(process.argv[2])fs.appendFileSync(process.argv[2],`called ${request.params.name}\n`);return request.params.name==="fail"?{isError:true,content:[{type:"text",text:"MCP fixture failed"}]}:{content:[{type:"text",text:`MCP fixture: ${request.params.arguments?.text??request.params.name}`}]};});
await server.connect(new StdioServerTransport());process.stdin.on("end",()=>process.exit(0));
