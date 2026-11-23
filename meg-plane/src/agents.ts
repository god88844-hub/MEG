// The TS plane schedules delegates; the Rust worker owns inference and tools.
import { spawn, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import {routeTask}from"./security-router";

export type AgentEvent={id:string;role:string;status:string;elapsed:number};
type Task={child:ChildProcess;workspace:string;started:number};
export class AgentManager{
  private active=new Map<string,Task>();
  private handoffs=new Map<string,number>();
  constructor(private limit=4){}
  async run(params:any,signal:AbortSignal,notify:(event:AgentEvent)=>void){
    const role=params.role??"researcher";if(!["planner","researcher","reviewer"].includes(role))throw new Error("Delegate role must be planner, researcher or reviewer; delegated tools are read-only");
    if(typeof params.task!=="string"||!params.task.trim()||params.task.length>32_000)throw new Error("Delegate needs a task (up to 32,000 characters)");
    const handoffId=String(params.worker?.handoff_id??randomUUID());const count=this.handoffs.get(handoffId)??0;const route=routeTask(null,params.task,count);if(route.kind==="root-thread")throw new Error(route.reason);this.handoffs.set(handoffId,count+1);if(this.handoffs.size>1024)this.handoffs.delete(this.handoffs.keys().next().value!);
    if(this.active.size>=this.limit)throw new Error(`Delegate limit reached (${this.limit}); wait for an active worker`);
    const executable=process.env.MEG_NATIVE_EXE;if(!executable||!fs.existsSync(executable))throw new Error("Native MEG worker executable is unavailable");
    if(signal.aborted)throw new Error("Operation cancelled");
    const id=`${role}-${randomUUID().slice(0,8)}`;const started=Date.now();
    const child=spawn(executable,["worker"],{stdio:["pipe","pipe","pipe"],windowsHide:true,env:{...process.env,MEG_PLANE_DISABLED:"1"}});
    child.stdin!.on("error",()=>{});this.active.set(id,{child,workspace:params.workspace,started});
    const report=(status:string)=>notify({id,role,status,elapsed:(Date.now()-started)/1000});report("running");
    let size=0;const chunks:Buffer[]=[];child.stdout!.on("data",(chunk:Buffer)=>{size+=chunk.length;if(size>2*1024*1024)child.kill();else chunks.push(chunk);});child.stderr!.on("data",()=>{});
    const stop=()=>child.kill();signal.addEventListener("abort",stop,{once:true});const timeout=setTimeout(stop,300_000);
    try{
      const result=new Promise<void>((resolve,reject)=>{child.once("error",()=>reject(new Error("Could not start native MEG worker")));child.once("close",code=>code===0?resolve():reject(new Error(signal.aborted?"Delegate cancelled":"Delegate failed or timed out")));});
      child.stdin!.end(JSON.stringify({...params.worker,workspace:params.workspace,goal:`Act as a ${role}. This delegated run is read-only. Return useful findings and evidence.\nTask: ${params.task}`,mode:"plan"}));
      await result;const payload=JSON.parse(Buffer.concat(chunks).toString("utf8"));if(typeof payload.answer!=="string")throw new Error(typeof payload.error==="string"?payload.error:"Native worker returned no answer");report("completed");return {...payload,id,role,message:`${role}: ${payload.answer}`};
    }catch(error){report(signal.aborted?"cancelled":"failed");throw error;}finally{clearTimeout(timeout);signal.removeEventListener("abort",stop);this.active.delete(id);}
  }
  cancelAll(){for(const task of this.active.values())task.child.kill();}
  status(){return [...this.active].map(([id,t])=>({id,workspace:t.workspace,status:"running",elapsed:(Date.now()-t.started)/1000}));}
}
