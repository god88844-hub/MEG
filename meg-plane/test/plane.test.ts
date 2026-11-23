import { describe, expect, test } from "bun:test";
import { ModelRouter, type ModelEntry } from "../src/router";
import { SessionTree } from "../src/session-tree";
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
});

describe("SessionTree", () => {
  test("append + fork from any node replays the path", () => {
    const t = new SessionTree();
    const a = t.append(null, "user", "build site");
    const b = t.append(a.id, "assistant", "plan ready");
    const c = t.append(b.id, "tool", "scaffold written");
    const path = t.forkFrom(b.id);
    expect(path.map((n) => n.id)).toEqual([a.id, b.id]);
    expect(c.parentId).toBe(b.id);
    expect(t.size).toBe(3);
  });
});

describe("JSON-RPC dispatch", () => {
  test("router.probeLocal discovers a deterministic offline fleet", async () => {
    const server = Bun.serve({
      hostname: "127.0.0.1", port: 0,
      fetch: () => Response.json({ models: [{ name: "qwen-fixture", size: 1000 }] }),
    });
    try {
      const fleet = (await handleRpc("router.probeLocal", { baseUrl: server.url.origin })) as ModelEntry[];
      expect(fleet.some((m) => m.id === "qwen-fixture")).toBeTrue();
      expect(fleet.filter((m) => m.kind === "cloud").every((m) => m.status === "not-configured")).toBeTrue();
    } finally { server.stop(true); }
  });
  test("unknown method errors cleanly", async () => {
    await expect(handleRpc("no/such/method", {})).rejects.toThrow("unknown method");
  });

  test("session create/append/fork via RPC", async () => {
    const { sessionId } = (await handleRpc("session.create", {})) as any;
    const n1 = (await handleRpc("session.append", { sessionId, parentId: null, kind: "user", content: "hi" })) as any;
    const n2 = (await handleRpc("session.append", { sessionId, parentId: n1.id, kind: "assistant", content: "hello" })) as any;
    const path = (await handleRpc("session.forkPath", { sessionId, nodeId: n2.id })) as any[];
    expect(path).toHaveLength(2);
  });
});
