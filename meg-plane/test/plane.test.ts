import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ModelRouter, type ModelEntry } from "../src/router";
import { SessionTree, TaskStateMachine, initThreeZoneWorkspace, parseProgressMd, writeProgressMd } from "../src/session-tree";
import { signExecutionReceipt } from "../src/types";
import { AgentManager, type AgentEvent } from "../src/agents";
import { handleRpc } from "../src/index";

describe("ModelRouter", () => {
  const fleet: ModelEntry[] = [
    { id: "qwen2.5-coder:7b", provider: "ollama", kind: "local", roles: ["coding"], status: "ready" },
    { id: "openai/gpt", provider: "openai", kind: "cloud", roles: ["coding"], status: "not-configured" },
  ];

  test("prefers ready local over not-configured cloud", () => {
    const router = new ModelRouter();
    fleet.forEach((m) => router.register(m));
    const d = router.route({ goal: "build a website", role: "coding" });
    expect(d.selected?.id).toBe("qwen2.5-coder:7b");
    expect(d.reason).toContain("local");
  });

  test("no ready model = null decision, never a fake pick", () => {
    const router = new ModelRouter();
    router.register({ ...fleet[0], status: "unreachable" });
    const d = router.route({ goal: "anything" });
    expect(d.selected).toBeNull();
  });

  test("infers vision role from the goal", () => {
    const router = new ModelRouter();
    router.register({ id: "moondream", provider: "ollama", kind: "local", roles: ["vision"], status: "ready" });
    const d = router.route({ goal: "inspect this screenshot image" });
    expect(d.selected?.id).toBe("moondream");
  });

  test("cloud fallback when no local is ready", () => {
    const router = new ModelRouter();
    router.register({ id: "qwen", provider: "ollama", kind: "local", roles: ["coding"], status: "unreachable" });
    router.register({ id: "deepseek/deepseek", provider: "deepseek", kind: "cloud", roles: ["coding"], status: "ready" });
    expect(router.route({ goal: "fix bug", role: "coding" }).selected?.id).toBe("deepseek/deepseek");
  });

  test("syncFromConfigDir reads providers.json honestly without faking cloud availability", () => {
    const cfgDir = fs.mkdtempSync(path.join(os.tmpdir(), "meg-router-cfg-"));
    try {
      fs.writeFileSync(
        path.join(cfgDir, "providers.json"),
        JSON.stringify({
          version: 1,
          active: "openai",
          profiles: [
            { id: "openai", base_url: "https://api.openai.com/v1", default_model: "gpt-4o", key_env: "FIXTURE_OPENAI_KEY" },
            { id: "anthropic", base_url: "https://api.anthropic.com/v1", default_model: "claude-sonnet" },
            { id: "ollama", base_url: "http://127.0.0.1:11434/v1", default_model: "qwen2.5-coder:7b" },
          ],
        }),
      );
      const router = new ModelRouter();
      const synced = router.syncFromConfigDir(cfgDir, { FIXTURE_OPENAI_KEY: "sk-fixture" });
      expect(synced.find((m) => m.id === "gpt-4o")?.status).toBe("ready");
      expect(synced.find((m) => m.id === "claude-sonnet")?.status).toBe("not-configured");
      expect(synced.find((m) => m.id === "qwen2.5-coder:7b")?.status).toBe("unreachable");
    } finally {
      fs.rmSync(cfgDir, { recursive: true, force: true });
    }
  });
});

describe("SessionTree", () => {
  test("append + fork from any node replays the path", () => {
    const t = new SessionTree();
    const a = t.append(null, "user", "build site");
    const b = t.append(a.id, "assistant", "plan ready");
    const c = t.append(b.id, "tool", "scaffold written");
    const pathNodes = t.forkFrom(b.id);
    expect(pathNodes.map((n) => n.id)).toEqual([a.id, b.id]);
    expect(c.parentId).toBe(b.id);
    expect(t.size).toBe(3);
  });

  test("persists atomically under .meg/sessions and reloads branches cleanly", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-session-tree-"));
    try {
      const t1 = SessionTree.open(ws, "sess-alpha");
      const n1 = t1.append(null, "user", "first prompt");
      const n2 = t1.append(n1.id, "assistant", "first reply");
      const n3 = t1.append(n1.id, "branch", "alt branch");
      expect(SessionTree.list(ws)).toEqual(["sess-alpha"]);

      const reloaded = SessionTree.open(ws, "sess-alpha");
      expect(reloaded.size).toBe(3);
      expect(reloaded.forkFrom(n2.id).map((n) => n.content)).toEqual(["first prompt", "first reply"]);
      expect(reloaded.forkFrom(n3.id).map((n) => n.content)).toEqual(["first prompt", "alt branch"]);
      expect(() => SessionTree.open(ws, "../escape")).toThrow("Invalid session identifier");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});

describe("AgentManager", () => {
  test("emits structured progress heartbeats and kills worker tree on cancellation", async () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-agent-mgr-"));
    const prevExe = process.env.MEG_NATIVE_EXE;
    try {
      process.env.MEG_NATIVE_EXE = process.execPath;
      const mgr = new AgentManager(2, 40);
      const events: AgentEvent[] = [];
      const abort = new AbortController();
      const started = Date.now();
      setTimeout(() => abort.abort(), 140);

      await expect(
        mgr.run(
          {
            role: "researcher",
            task: "summarize workspace architecture",
            workspace: ws,
            _testArgs: ["-e", "setInterval(() => {}, 1000)"],
          },
          abort.signal,
          (ev) => events.push(ev),
        ),
      ).rejects.toThrow("cancelled");

      expect(Date.now() - started).toBeLessThan(3000);
      expect(events[0]?.phase).toBe("starting");
      expect(events.some((e) => e.phase === "working")).toBeTrue();
      expect(events.at(-1)?.status).toBe("cancelled");
      expect(events.at(-1)?.phase).toBe("done");
      expect(mgr.status()).toHaveLength(0);
    } finally {
      if (prevExe === undefined) delete process.env.MEG_NATIVE_EXE;
      else process.env.MEG_NATIVE_EXE = prevExe;
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});

describe("JSON-RPC dispatch", () => {
  test("router.probeLocal discovers a deterministic offline fleet", async () => {
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: () => Response.json({ models: [{ name: "qwen-fixture", size: 1000 }] }),
    });
    try {
      const fleet = (await handleRpc("router.probeLocal", { baseUrl: server.url.origin })) as ModelEntry[];
      expect(fleet.some((m) => m.id === "qwen-fixture")).toBeTrue();
      expect(fleet.filter((m) => m.kind === "cloud").every((m) => m.status === "not-configured")).toBeTrue();
    } finally {
      server.stop(true);
    }
  });

  test("unknown method errors cleanly", async () => {
    await expect(handleRpc("no/such/method", {})).rejects.toThrow("unknown method");
  });

  test("session create/append/fork via RPC", async () => {
    const { sessionId } = (await handleRpc("session.create", {})) as any;
    const n1 = (await handleRpc("session.append", { sessionId, parentId: null, kind: "user", content: "hi" })) as any;
    const n2 = (await handleRpc("session.append", { sessionId, parentId: n1.id, kind: "assistant", content: "hello" })) as any;
    const pathNodes = (await handleRpc("session.forkPath", { sessionId, nodeId: n2.id })) as any[];
    expect(pathNodes).toHaveLength(2);
  });

  test("persistent session list/load/fork via RPC", async () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-rpc-sess-"));
    try {
      const created = (await handleRpc("session.create", { workspace: ws, sessionId: "persist-1" })) as any;
      expect(created.persisted).toBeTrue();
      const n1 = (await handleRpc("session.append", {
        workspace: ws,
        sessionId: "persist-1",
        parentId: null,
        kind: "user",
        content: "hello disk",
      })) as any;
      const listed = (await handleRpc("session.list", { workspace: ws })) as any;
      expect(listed.sessions).toContain("persist-1");
      const loaded = (await handleRpc("session.load", { workspace: ws, sessionId: "persist-1" })) as any;
      expect(loaded.size).toBe(1);
      expect(loaded.nodes[0].id).toBe(n1.id);
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});
describe("P2-02 TaskStateMachine, 3-Zone Workspace & Crash Replay", () => {
  test("enforces deterministic transitions, requires signed receipt for VERIFIED, and replays from ledger", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p202-"));
    try {
      const zones = initThreeZoneWorkspace(ws, "Build P2-02");
      expect(fs.existsSync(zones.sources)).toBeTrue();
      expect(fs.existsSync(zones.drafts)).toBeTrue();
      expect(fs.existsSync(zones.published)).toBeTrue();
      const parsed = parseProgressMd(fs.readFileSync(zones.progressFile, "utf8"));
      expect(parsed.task).toBe("Build P2-02");

      writeProgressMd(zones.progressFile, {
        task: "Build P2-02",
        outputs: ["src/session-tree.ts"],
        completed: ["Created zones"],
        decisions: ["Use append-only JSONL ledger"],
        openIssues: [],
        nextAction: "Run bun test",
      });
      expect(parseProgressMd(fs.readFileSync(zones.progressFile, "utf8")).outputs).toEqual(["src/session-tree.ts"]);

      const ledger = path.join(ws, ".meg", "tasks", "t-1.events.jsonl");
      const sm = new TaskStateMachine("t-1", ledger);
      expect(() => sm.transition("VERIFIED")).toThrow("Invalid task state transition");
      sm.transition("SCOPED");
      sm.transition("PLANNED");
      sm.transition("EXECUTING");
      sm.transition("VERIFYING");

      // VERIFIED blocked without valid receipt
      expect(() => sm.transition("VERIFIED")).toThrow("machine-verified receipt");

      // Machine-verified receipt commits VERIFIED
      const receipt = signExecutionReceipt("t-1", "tree-sha-1", true);
      sm.transition("VERIFIED", { receipt, expectedTreeHash: "tree-sha-1" });
      expect(sm.state).toBe("VERIFIED");

      // Crash/restart replay restores VERIFIED state deterministically
      const replayed = TaskStateMachine.replayFromLedger("t-1", ledger);
      expect(replayed.state).toBe("VERIFIED");
      expect(replayed.history().length).toBe(sm.history().length);
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});