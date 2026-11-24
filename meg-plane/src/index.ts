// JSON-RPC 2.0 over stdio -- the control-plane seam for the native Rust TUI,
// workspace extensions, MCP servers and native worker delegation.

import { ModelRouter, type ModelEntry, type StoredProviderProfile } from "./router";
import { SessionTree, TaskStateMachine, initThreeZoneWorkspace, type TaskState } from "./session-tree";
import { McpHost } from "./mcp-host";
import { WorkspaceExtensions, configRoot } from "./extensions";
import { AgentManager } from "./agents";
import { dispatchSlashCommand, dropBacklogTicket, routeMagnitude, type PstackMode } from "./commands";
import { evaluateAmbiguityGate, runTurn0Bootstrap } from "./bootstrap";
import { buildCompressedHandoff, compactProcessedMedia, compileContext } from "./context";
import { checkEpamCircuitBreaker, classifyBudgetProfile, evaluateAgentBaton, selectHarnessGear } from "./fusion";
import { BiTemporalMemoryStore, lexicalSearchWorkspace, stairTocLeafRetrieve } from "./memory";
import { OmnigraphStore } from "./omnigraph";
import { scanSkillSupplyChain } from "./security-router";
import { loadSkillFromDir, renderSkillPlusPlusIndex, selectActiveSkills } from "./skills";
import { routeCliOrMcp } from "./mcp-host";
import { assertVerificationLevel, evaluateMirlGate } from "./checks_gate";
import { assignColdCrossModelCritic } from "./critic";
import { formatRemediationLint, scanAntiStubAndFalseZero } from "./poteto_checks";
import { evaluateEarlyStartIndependence } from "./graphs";
import { buildUnitOnlyReturnEdge } from "./agents";
import { GoalManager, checkAntiGoalGamingInvariants } from "./goals_motions";
import { compileIssueContractAndBreakTrifecta, triageCiFailure } from "./github";
import { ingestClipboardMedia, ingestDroppedMedia } from "./media";
import fs from "node:fs";
import path from "node:path";

const router = new ModelRouter();
const sessions = new Map<string, SessionTree>();
const tasks = new Map<string, TaskStateMachine>();
const memories = new Map<string, BiTemporalMemoryStore>();
const omnigraphs = new Map<string, OmnigraphStore>();
const goals = new Map<string, GoalManager>();

function getMemoryStore(workspace: string): BiTemporalMemoryStore {
  const key = fs.realpathSync(workspace);
  let store = memories.get(key);
  if (!store) {
    store = new BiTemporalMemoryStore(path.join(key, ".meg", "memory.jsonl"));
    memories.set(key, store);
  }
  return store;
}

function getOmnigraph(workspace: string): OmnigraphStore {
  const key = fs.realpathSync(workspace);
  let og = omnigraphs.get(key);
  if (!og) {
    og = new OmnigraphStore();
    omnigraphs.set(key, og);
  }
  return og;
}

function getGoalManager(workspace: string): GoalManager {
  const key = fs.realpathSync(workspace);
  let gm = goals.get(key);
  if (!gm) {
    gm = new GoalManager(key);
    goals.set(key, gm);
  }
  return gm;
}
const mcp = new McpHost();
const workspaces = new Map<string, WorkspaceExtensions>();
const delegates = new AgentManager(4);
const activeRequests = new Map<string | number, { abort: AbortController; workspace?: string; method: string }>();

type RpcContext = { signal: AbortSignal; notify: (event: unknown) => void };

async function workspaceRuntime(workspace: string) {
  if (typeof workspace !== "string" || !fs.statSync(workspace).isDirectory()) {
    throw new Error("Workspace directory is unavailable");
  }
  const key = fs.realpathSync(workspace);
  let runtime = workspaces.get(key);
  if (!runtime) {
    runtime = new WorkspaceExtensions(key);
    workspaces.set(key, runtime);
    await runtime.reconnectTrusted();
  }
  return runtime;
}

function sessionCacheKey(sessionId: string, workspace?: string): string {
  return workspace ? `${fs.realpathSync(workspace)}\0${sessionId}` : sessionId;
}

function getOrOpenSession(sessionId: string, workspace?: string): SessionTree {
  const key = sessionCacheKey(sessionId, workspace);
  let tree = sessions.get(key) ?? sessions.get(sessionId);
  if (!tree && workspace) {
    tree = SessionTree.open(workspace, sessionId);
    sessions.set(key, tree);
  }
  if (!tree) throw new Error(`no session ${sessionId}`);
  return tree;
}

type Handler = (params: any) => Promise<unknown> | unknown;

const handlers: Record<string, Handler> = {
  "router.probeLocal": async (p: { baseUrl?: string }) => {
    const models: ModelEntry[] = await ModelRouter.probeLocal(p?.baseUrl);
    for (const m of models) router.register(m);
    for (const m of ModelRouter.cloudLadder()) router.register(m); // honest not-configured
    return router.fleet();
  },
  "router.syncProfiles": (p: { profiles?: StoredProviderProfile[] }) =>
    router.syncProfiles(Array.isArray(p?.profiles) ? p.profiles : []),
  "router.syncConfig": (p: { configDir?: string }) =>
    router.syncFromConfigDir(p?.configDir ?? configRoot()),
  "router.route": (p: { goal: string; role?: string }) => router.route(p),
  "session.create": (p?: { workspace?: string; sessionId?: string }) => {
    const id = p?.sessionId ?? `s-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
    if (p?.workspace) {
      const tree = SessionTree.open(p.workspace, id);
      sessions.set(sessionCacheKey(id, p.workspace), tree);
      return { sessionId: tree.sessionId, persisted: true };
    }
    const tree = new SessionTree({ sessionId: id });
    sessions.set(id, tree);
    return { sessionId: tree.sessionId, persisted: false };
  },
  "session.list": (p: { workspace: string }) => ({
    sessions: SessionTree.list(p.workspace),
  }),
  "session.load": (p: { workspace: string; sessionId: string }) => {
    const tree = SessionTree.open(p.workspace, p.sessionId);
    sessions.set(sessionCacheKey(tree.sessionId, p.workspace), tree);
    return { sessionId: tree.sessionId, nodes: tree.allNodes(), size: tree.size };
  },
  "session.append": (p: {
    sessionId: string;
    workspace?: string;
    parentId: string | null;
    kind: string;
    content: string;
  }) => {
    const tree = getOrOpenSession(p.sessionId, p.workspace);
    return tree.append(p.parentId, p.kind as any, p.content);
  },
  "session.forkPath": (p: { sessionId: string; workspace?: string; nodeId: string }) => {
    const tree = getOrOpenSession(p.sessionId, p.workspace);
    return tree.forkFrom(p.nodeId);
  },
  "workspace.initZones": (p: { workspace: string; task?: string }) =>
    initThreeZoneWorkspace(p.workspace, p.task ?? ""),
  "task.create": (p: { taskId: string; workspace?: string }) => {
    const ledger = p.workspace
      ? path.join(fs.realpathSync(p.workspace), ".meg", "tasks", `${p.taskId}.events.jsonl`)
      : null;
    const sm = new TaskStateMachine(p.taskId, ledger);
    tasks.set(p.taskId, sm);
    return { taskId: sm.taskId, state: sm.state };
  },
  "task.transition": (p: { taskId: string; to: TaskState; reason?: string; receipt?: any; expectedTreeHash?: string }) => {
    const sm = tasks.get(p.taskId);
    if (!sm) throw new Error(`unknown task: ${p.taskId}`);
    const ev = sm.transition(p.to, { reason: p.reason, receipt: p.receipt, expectedTreeHash: p.expectedTreeHash });
    return { taskId: sm.taskId, state: sm.state, event: ev };
  },
  "task.status": (p: { taskId: string }) => {
    const sm = tasks.get(p.taskId);
    if (!sm) throw new Error(`unknown task: ${p.taskId}`);
    return { taskId: sm.taskId, state: sm.state, events: sm.history().length, verified: sm.state === "VERIFIED" };
  },
  "task.replay": (p: { taskId: string; workspace: string }) => {
    const ledger = path.join(fs.realpathSync(p.workspace), ".meg", "tasks", `${p.taskId}.events.jsonl`);
    const sm = TaskStateMachine.replayFromLedger(p.taskId, ledger);
    tasks.set(p.taskId, sm);
    return { taskId: sm.taskId, state: sm.state, events: sm.history().length };
  },
  "commands.routeMagnitude": (p: { files: number; complexity?: "trivial" | "normal" | "complex" | "mega" }) =>
    routeMagnitude(p.files, p.complexity),
  "commands.dispatch": (p: { input: string; workspace: string; sessionEditedFiles?: string[]; currentMode?: PstackMode }) =>
    dispatchSlashCommand(p.input, {
      workspace: p.workspace,
      sessionEditedFiles: p.sessionEditedFiles ?? [],
      currentMode: p.currentMode ?? "feature",
    }),
  "commands.backlogDrop": (p: { workspace: string; title: string; reason?: string }) => ({
    file: dropBacklogTicket(p.workspace, p.title, p.reason),
  }),
  "mcp.startServer": async (p: { name: string; command: string; args?: string[] }) =>
    mcp.startServer(p.name, p.command, p.args ?? []),
  "mcp.callTool": (p: { server: string; tool: string; args: Record<string, unknown> }) =>
    mcp.callTool(p.server, p.tool, p.args),
  "mcp.allTools": () => mcp.allTools(),
  "mcp.routePolicy": (p: { target: string; extraAllowedMcp?: string[] }) =>
    routeCliOrMcp(p.target, { extraAllowedMcp: p.extraAllowedMcp }),
  "bootstrap.probe": (p: { workspace: string }) => runTurn0Bootstrap(p.workspace),
  "bootstrap.ambiguityGate": (p: { workspace: string; candidates: any[] }) =>
    evaluateAmbiguityGate(p.workspace, p.candidates ?? []),
  "context.compile": (p: any) => compileContext(p),
  "context.handoff": (p: any) => ({ brief: buildCompressedHandoff(p) }),
  "context.compactMedia": (p: { workspace: string; blocks: any[] }) =>
    compactProcessedMedia(p.workspace, p.blocks ?? []),
  "fusion.classifyBudget": (p: { profile: any; confidence: number }) =>
    classifyBudgetProfile(p.profile, p.confidence),
  "fusion.selectGear": (p: { paramBillions: number | "frontier" }) =>
    selectHarnessGear(p.paramBillions),
  "fusion.checkEpam": (p: any) => checkEpamCircuitBreaker(p),
  "fusion.baton": (p: any) => evaluateAgentBaton(p),
  "memory.record": (p: { workspace: string; key: string; value: string; epistemic: any; sourceFile?: string }) =>
    getMemoryStore(p.workspace).recordFact(p),
  "memory.query": (p: { workspace: string; key: string; validAt?: number }) =>
    getMemoryStore(p.workspace).queryAsOf(p.key, p.validAt ?? Date.now()),
  "memory.lexicalSearch": (p: { workspace: string; pattern: string; maxHits?: number }) =>
    lexicalSearchWorkspace(p.workspace, p.pattern, p.maxHits),
  "memory.stairRetrieve": (p: { markdown: string; query: string; maxLeaves?: number }) =>
    stairTocLeafRetrieve(p.markdown, p.query, p.maxLeaves),
  "omnigraph.branch": (p: { workspace: string; branch: string; from?: string }) => {
    getOmnigraph(p.workspace).createBranch(p.branch, p.from ?? "main");
    return { created: p.branch };
  },
  "omnigraph.put": (p: { workspace: string; branch: string; slug: string; value: string; actor: string; expectedBaseVersion?: number }) =>
    getOmnigraph(p.workspace).put(p.branch, p.slug, p.value, p.actor, p.expectedBaseVersion),
  "omnigraph.get": (p: { workspace: string; branch: string; slug: string; snapshot?: number }) =>
    getOmnigraph(p.workspace).get(p.branch, p.slug, { snapshot: p.snapshot }),
  "omnigraph.merge": (p: { workspace: string; branch: string; role: "worker" | "reviewer"; actor?: string }) =>
    getOmnigraph(p.workspace).mergeToMain(p.branch, p.role, p.actor),
  "skills.scanDdipe": (p: { content: string; source?: string }) =>
    scanSkillSupplyChain(p.content, p.source),
  "skills.selectActive": (p: { skillDirs: string[]; query: string }) => {
    const loaded = (p.skillDirs ?? []).map((d) => loadSkillFromDir(d, { thirdParty: true }));
    const res = selectActiveSkills(loaded, p.query);
    return { selected: res.selected.map((s) => s.metadata), droppedReasons: res.droppedReasons, index: renderSkillPlusPlusIndex(res.selected) };
  },
  "verifier.checkLevel": (p: { level: any }) => assertVerificationLevel(p.level),
  "verifier.mirlGate": (p: any) => evaluateMirlGate(p),
  "verifier.potetoScan": (p: { filePath: string; source: string }) => {
    const findings = scanAntiStubAndFalseZero(p.source);
    return { ok: findings.length === 0, findings, formatted: formatRemediationLint(p.filePath, findings) };
  },
  "verifier.assignCritic": (p: { writer: any; pool: any[] }) =>
    assignColdCrossModelCritic(p.writer, p.pool ?? []),
  "graphs.earlyStart": (p: { slices: any[] }) => evaluateEarlyStartIndependence(p.slices ?? []),
  "graphs.unitReturnEdge": (p: { slices: any[] }) => buildUnitOnlyReturnEdge(p.slices ?? []),
  "goals.handoff": (p: { workspace: string; spec: any; budget: { maxUsd: number; maxRuns: number } }) =>
    getGoalManager(p.workspace).handoffFromState(p.spec, p.budget),
  "goals.command": (p: { workspace: string; command: string }) =>
    getGoalManager(p.workspace).handleCommand(p.command),
  "goals.checkInvariants": (p: any) => checkAntiGoalGamingInvariants(p),
  "github.compileIssueContract": (p: { issue: any; plan: any }) =>
    compileIssueContractAndBreakTrifecta(p.issue, p.plan),
  "github.triageCi": (p: { logText: string }) => triageCiFailure(p.logText),
  "media.ingestDrop": (p: { workspace: string; rawPath: string }) =>
    ingestDroppedMedia(p.workspace, p.rawPath),
  "media.ingestClipboard": (p: { workspace: string; payload: string; mimeType?: string }) =>
    ingestClipboardMedia(p.workspace, p.payload, p.mimeType),
};

export async function handleRpc(
  method: string,
  params: any,
  context: RpcContext = { signal: new AbortController().signal, notify: () => {} },
): Promise<unknown> {
  if (method === "runtime.ping") return { ready: true, version: "0.1.0" };
  if (method === "runtime.cancel") {
    const request = activeRequests.get(params.requestId);
    request?.abort.abort();
    if (request?.workspace && request.method === "mcp.command") {
      const runtime = workspaces.get(fs.realpathSync(request.workspace));
      await runtime?.host.stopAll();
    }
    return { cancelled: true };
  }
  if (method === "agents.status") return { agents: delegates.status() };
  if (method === "agents.run") return delegates.run(params, context.signal, context.notify);
  if (
    method.startsWith("runtime.") ||
    method === "skills.read" ||
    method === "plugins.command" ||
    method === "mcp.command"
  ) {
    const runtime = await workspaceRuntime(params.workspace);
    if (method === "runtime.catalog") {
      return { catalog: await runtime.catalog(), message: "Skills, plugins and MCP catalog refreshed" };
    }
    if (method === "skills.read") return runtime.readSkill(params.name);
    if (method === "plugins.command") return runtime.pluginCommand(params.command ?? "");
    if (method === "mcp.command") return runtime.mcpCommand(params.command ?? "");
    if (method === "runtime.tool") {
      return runtime.tool(params.tool, params.args ?? {}, !!params.readOnly, context.signal);
    }
  }
  const handler = handlers[method];
  if (!handler) throw new Error(`unknown method: ${method}`);
  return handler(params);
}

// stdio JSON-RPC 2.0 dispatch loop
if (import.meta.main) {
  let buf = "";
  const respond = (value: unknown) => {
    const line = JSON.stringify(value);
    if (Buffer.byteLength(line) > 4 * 1024 * 1024) {
      throw new Error("Control-plane result exceeds 4 MiB");
    }
    process.stdout.write(line + "\n");
  };
  const dispatch = async (msg: any) => {
    if (!msg || typeof msg !== "object" || typeof msg.method !== "string") return;
    const id = msg.id;
    const abort = new AbortController();
    if (id !== undefined) {
      activeRequests.set(id, { abort, workspace: msg.params?.workspace, method: msg.method });
    }
    try {
      const result = await handleRpc(msg.method, msg.params ?? {}, {
        signal: abort.signal,
        notify: (event) =>
          respond({ jsonrpc: "2.0", method: "agent.event", params: { ...(event as object), requestId: id } }),
      });
      if (id !== undefined) respond({ jsonrpc: "2.0", id, result });
    } catch (error) {
      if (id !== undefined) {
        respond({
          jsonrpc: "2.0",
          id,
          error: { code: -32603, message: error instanceof Error ? error.message : "Control-plane operation failed" },
        });
      }
    } finally {
      if (id !== undefined) activeRequests.delete(id);
    }
  };
  process.stdin.setEncoding("utf-8");
  process.stdin.on("data", (chunk: string) => {
    buf += chunk;
    if (Buffer.byteLength(buf) > 4 * 1024 * 1024) {
      process.stderr.write("Control-plane input exceeded limit\n");
      process.exit(1);
    }
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
      if (!msg || typeof msg !== "object") continue;
      void dispatch(msg);
    }
  });
  const shutdown = async () => {
    for (const request of activeRequests.values()) request.abort.abort();
    delegates.cancelAll();
    await Promise.all([mcp.stopAll(), ...[...workspaces.values()].map((r) => r.host.stopAll())]);
  };
  process.stdin.on("end", () => void shutdown().finally(() => process.exit(0)));
  process.on("SIGTERM", () => void shutdown().finally(() => process.exit(0)));
  process.stderr.write("meg-plane control plane ready (JSON-RPC 2.0 over stdio)\n");
}