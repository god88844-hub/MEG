import {afterAll,describe,expect,test}from"bun:test";
import fs from"node:fs";import os from"node:os";import path from"node:path";
import {McpHost}from"../src/mcp-host";import{WorkspaceExtensions}from"../src/extensions";
const root=fs.mkdtempSync(path.join(os.tmpdir(),"meg-extensions-test-"));const ws=path.join(root,"workspace");fs.mkdirSync(ws);const config=path.join(root,"config");
const oldConfig=process.env.MEG_CONFIG_DIR;process.env.MEG_CONFIG_DIR=config;
const server=path.join(import.meta.dir,"fixtures/mcp-server.ts");const marker=path.join(root,"mcp-starts.txt");
const runtime=new WorkspaceExtensions(ws);const host=new McpHost();
afterAll(async()=>{await Promise.all([host.stopAll(),runtime.host.stopAll()]);if(oldConfig===undefined)delete process.env.MEG_CONFIG_DIR;else process.env.MEG_CONFIG_DIR=oldConfig;const resolved=path.resolve(root);if(!resolved.startsWith(path.resolve(os.tmpdir())+path.sep)||!path.basename(resolved).startsWith("meg-extensions-test-"))throw new Error("Unsafe cleanup path");fs.rmSync(resolved,{recursive:true,force:true});});
describe("real MCP lifecycle",()=>{
  test("initializes, discovers schemas, calls and closes a real stdio server",async()=>{const tools=await host.startServer("fixture",process.execPath,[server]);expect(tools[0].readOnly).toBeTrue();expect(tools[0].inputSchema.type).toBe("object");expect(JSON.stringify(await host.callTool("fixture","echo",{text:"hello"}))).toContain("MCP fixture: hello");await host.stopAll();expect(host.allTools()).toHaveLength(0);});
  test("does not run untrusted workspace MCP configuration just to list it",async()=>{fs.writeFileSync(path.join(ws,"mcp_config.json"),JSON.stringify({mcpServers:{fixture:{command:process.execPath,args:[server,marker]}}}));expect((await runtime.catalog()).servers[0].connected).toBeFalse();expect(fs.existsSync(marker)).toBeFalse();});
  test("explicit start connects tools and Plan enforces annotations",async()=>{await runtime.mcpCommand("start fixture");expect(fs.existsSync(marker)).toBeTrue();expect(JSON.stringify(await runtime.tool("mcp:fixture:echo",{text:"connected"},true))).toContain("connected");try{await runtime.tool("mcp:fixture:mutate",{},true);throw new Error("unexpected success")}catch(e){expect(String(e)).toContain("Plan mode")};expect(JSON.stringify(await runtime.tool("mcp:fixture:mutate",{},false))).toContain("mutate");try{await runtime.tool("mcp:fixture:fail",{},false);throw new Error("unexpected success")}catch(e){expect(String(e)).toContain("MCP fixture failed")};});
  test("trusted unchanged configuration reconnects; changed commands do not",async()=>{await runtime.host.stopAll();const reopened=new WorkspaceExtensions(ws);await reopened.reconnectTrusted();expect(reopened.host.allTools()).toHaveLength(3);await reopened.host.stopAll();fs.writeFileSync(path.join(ws,"mcp_config.json"),JSON.stringify({mcpServers:{fixture:{command:"changed-command",args:[]}}}));const changed=new WorkspaceExtensions(ws);await changed.reconnectTrusted();expect(changed.host.allTools()).toHaveLength(0);});
});
describe("skill and plugin data paths",()=>{
  test("a missing plugin manifest is reported and its registration remains removable",async()=>{const directory=path.join(root,"missing-plugin");fs.mkdirSync(directory);const manifest=path.join(directory,"meg-plugin.json");fs.writeFileSync(manifest,JSON.stringify({name:"missing-plugin"}));await runtime.pluginCommand(`add ${directory}`);fs.unlinkSync(manifest);expect((await runtime.catalog()).plugins.find(p=>p.name==="missing-plugin")?.error).toContain("needs");await runtime.pluginCommand("remove missing-plugin");expect((await runtime.catalog()).plugins.some(p=>p.name==="missing-plugin")).toBeFalse();});
  test("loads actual SKILL.md bodies and plugin command templates",async()=>{const skillDir=path.join(ws,".meg/skills/offline-skill");fs.mkdirSync(skillDir,{recursive:true});fs.writeFileSync(path.join(skillDir,"SKILL.md"),"---\nname: offline-skill\ndescription: Offline workflow\n---\nCheck the fixture evidence first.");const plugin=path.join(root,"plugin");fs.mkdirSync(path.join(plugin,"commands"),{recursive:true});fs.writeFileSync(path.join(plugin,"meg-plugin.json"),JSON.stringify({name:"fixture-plugin"}));fs.writeFileSync(path.join(plugin,"commands/check.md"),"Verify $ARGUMENTS using local evidence.");const result=await runtime.pluginCommand(`add ${plugin}`);expect(result.catalog.commands[0].name).toBe("fixture-plugin.check");expect((await runtime.readSkill("offline-skill")).body).toContain("fixture evidence");await runtime.pluginCommand("disable fixture-plugin");expect((await runtime.catalog()).commands).toHaveLength(0);await runtime.pluginCommand("enable fixture-plugin");expect((await runtime.catalog()).commands).toHaveLength(1);await runtime.pluginCommand("remove fixture-plugin");expect(fs.existsSync(path.join(plugin,"meg-plugin.json"))).toBeTrue();});
});

import {
  DecisionGraphCache,
  evaluateAfterToolRisks,
  evaluatePreToolUseHook,
  resolveLazyToolImport,
} from "../src/extensions";

describe("P2-07 Code Mode, HarnessMM & 4-Stage / 7-Risk Hooks", () => {
  test("resolves lazy @tools/* typed imports and rejects non-tool specifiers", () => {
    const desc = resolveLazyToolImport("@tools/edit_file");
    expect(desc.loaded).toBeTrue();
    expect(desc.typedSignature).toContain("editFile");
    expect(() => resolveLazyToolImport("fs")).toThrow("@tools/*");
  });

  test("HarnessMM GDR cache enforces Substitution Paradox before bypassing reasoning", () => {
    const cache = new DecisionGraphCache();
    cache.register({
      id: "edg-lint",
      triggerPattern: "run lint",
      toolSequence: ["bash"],
      preconditions: ["clean_git"],
      cachedAt: Date.now(),
      hitCount: 0,
    });

    // Rejected when overhead exceeds eliminated reasoning
    const denied = cache.lookupGdr("please run lint", ["clean_git"], {
      eliminatedReasoningTokens: 400,
      authorshipCostTokens: 200,
      selectionCostTokens: 150,
      recoveryCostTokens: 100,
    });
    expect(denied.hit).toBeFalse();
    expect(denied.certificate?.approved).toBeFalse();

    // Approved when savings strictly exceed overhead
    const hit = cache.lookupGdr("please run lint", ["clean_git"], {
      eliminatedReasoningTokens: 2000,
      authorshipCostTokens: 200,
      selectionCostTokens: 150,
      recoveryCostTokens: 100,
    });
    expect(hit.hit).toBeTrue();
    expect(hit.ccsScore).toBe(1);
    expect(hit.node?.hitCount).toBe(1);
  });

  test("HARNESSPRO 4-stage PreToolUse hook uses absolute $MEG_HOME and AHE 7-risk AfterToolHook promotes reminders", () => {
    const budget = { econ: 6, strict: 10, final: 12, rem: 4 }; // grace = max(8, 4) = 8 -> hard limit 20
    const env = { MEG_HOME: process.platform === "win32" ? "C:\\meg-home" : "/opt/meg-home" };

    const s1 = evaluatePreToolUseHook(2, budget, "read_file", env);
    expect(s1.stage).toBe("economy");
    expect(s1.allow).toBeTrue();
    expect(s1.hookPath.startsWith(env.MEG_HOME)).toBeTrue();

    expect(evaluatePreToolUseHook(7, budget, "edit_file", env).stage).toBe("convergence");
    expect(evaluatePreToolUseHook(14, budget, "bash", env).stage).toBe("finalization_grace");
    const s4 = evaluatePreToolUseHook(20, budget, "bash", env);
    expect(s4.stage).toBe("hard_stop_deny");
    expect(s4.allow).toBeFalse();
    // `finish` remains permitted even at hard limit
    expect(evaluatePreToolUseHook(20, budget, "finish", env).allow).toBeTrue();

    const risk = evaluateAfterToolRisks({
      toolName: "edit_file",
      argsText: "test.skip('flaky'); token=sk-proj-1234567890abcdef1234567890",
      outputText: "stale hashline anchor",
      consecutiveRepeats: 3,
      editedOutsideScope: true,
    });
    expect(risk.risks).toContain("secret_exposure");
    expect(risk.risks).toContain("test_weakening");
    expect(risk.risks).toContain("scope_creep");
    expect(risk.risks).toContain("stale_context");
    expect(risk.risks).toContain("infinite_retry");
    expect(risk.topOfTurnReminder).toContain("[FRAMEWORK REMINDER]");
  });
});