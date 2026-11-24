import { describe, expect, test } from "bun:test";
import {
  GlobalRouter,
  LoopUntilDrySeen,
  PlanfenceCache,
  TeamCreateCoordinator,
  assertIndependentDebateProposals,
  buildUnitOnlyReturnEdge,
  classifySwarmShape,
} from "../src/agents";

describe("P5-02 DeepSeek Router-Worker, 4-Shape Swarm & Return Edges", () => {
  test("GlobalRouter is strictly Bash-less, batches prefill triggers, passes thread subscriptions directly, and supports full-duplex start/stop", () => {
    const router = new GlobalRouter();
    expect(router.bashEnabled).toBeFalse();
    expect(() => router.assertToolAllowed("bash")).toThrow("Bash-less");

    const received: string[] = [];
    router.subscribeThread("t-1", (m) => received.push(m.payload));
    router.start();
    router.dispatchTrigger({ id: "1", threadId: "t-1", payload: "ping" });
    router.dispatchTrigger({ id: "2", threadId: "t-2", payload: "pong" });
    expect(received).toEqual(["ping"]);
    expect(router.flushPrefillBatch()).toHaveLength(2);
    router.stop();
    expect(router.isRunning).toBeFalse();
  });

  test("4-Shape classifier, DEBATE independence, TeamCreate 1-File-1-Owner locks (<=5) & early shutdown", () => {
    const fanout = classifySwarmShape({ independentItems: 25 });
    expect(fanout.shape).toBe("FAN-OUT");
    expect(fanout.maxItemsPerWorker!).toBeLessThanOrEqual(10);

    expect(classifySwarmShape({ needsAdversarialConsensus: true }).shape).toBe("DEBATE");
    expect(() =>
      assertIndependentDebateProposals([
        { agentId: "a", proposal: "p1", sawPeerOutput: false },
        { agentId: "b", proposal: "p2", sawPeerOutput: true },
      ]),
    ).toThrow("DEBATE_VIOLATION");

    const team = new TeamCreateCoordinator();
    const mkHandoff = (id: string, file: string) => ({
      handoffId: id,
      role: "builder" as const,
      objective: `Edit ${file}`,
      allowedFiles: [file],
      constraints: ["stay in fence"],
      acceptanceCriteria: ["tests pass"],
      budget: { maxTurns: 5, maxTokens: 8000 },
    });

    team.spawnTeammate("w1", mkHandoff("h1", "src/a.ts"));
    expect(() => team.spawnTeammate("w2", mkHandoff("h2", "src/a.ts"))).toThrow("FILE_LOCK_CONFLICT");

    // Early shutdown releases file lock so w2 can acquire it cleanly
    team.shutdownTeammate("w1");
    expect(team.activeTeammates()).toHaveLength(0);
    team.spawnTeammate("w2", mkHandoff("h2", "src/a.ts"));
    expect(team.ownerOf("src/a.ts")).toBe("w2");
  });

  test("Unit-Only Return Edge freezes passed slices, Long Learning Edge feeds Splitter, Planfence hashes inputs, and Loop-Until-Dry dedupes seen", () => {
    const ret = buildUnitOnlyReturnEdge([
      { unitId: "U1", files: ["src/auth.ts"], passed: true },
      { unitId: "U2", files: ["src/billing.ts"], passed: false, failureSummary: "tax rounding off by 1c" },
    ]);
    expect(ret.frozenReadOnlyFiles).toEqual(["src/auth.ts"]);
    expect(ret.retryUnits).toHaveLength(1);
    expect(ret.retryUnits[0]?.unitId).toBe("U2");
    expect(ret.retryUnits[0]?.fileFence).toEqual(["src/billing.ts"]);
    expect(ret.longLearningNotesForSplitter[0]).toContain("U2");

    const pf = new PlanfenceCache();
    expect(pf.needsRebuild("plan-1", { spec: "v1" }).rebuild).toBeTrue();
    expect(pf.needsRebuild("plan-1", { spec: "v1" }).rebuild).toBeFalse();
    expect(pf.needsRebuild("plan-1", { spec: "v2" }).rebuild).toBeTrue();

    const seen = new LoopUntilDrySeen();
    seen.markConfirmed("item-1");
    seen.markRejected("item-2");
    expect(seen.filterUnseen(["item-1", "item-2", "item-3"], (x) => x)).toEqual(["item-3"]);
  });
});