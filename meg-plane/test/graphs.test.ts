import { describe, expect, test } from "bun:test";
import {
  Blackboard,
  SagaStack,
  evaluateEarlyStartIndependence,
  parallel,
  partitionParentBudget,
  pipeline,
  recurse,
  routeNodeContract,
  sequential,
  verify,
} from "../src/graphs";

describe("P5-01 Algebraic Orchestration (Maestro Order) & Code-Routed DAG", () => {
  test("defaults to single-agent unless Early-Start independence passes with disjoint writePaths", () => {
    expect(evaluateEarlyStartIndependence([{ id: "s1", writePaths: ["a.ts"] }]).mode).toBe("single_agent");

    const overlap = evaluateEarlyStartIndependence([
      { id: "s1", writePaths: ["src/shared.ts"] },
      { id: "s2", writePaths: ["src/shared.ts"] },
    ]);
    expect(overlap.mode).toBe("single_agent");
    expect(overlap.independent).toBeFalse();

    const disjoint = evaluateEarlyStartIndependence([
      { id: "s1", writePaths: ["src/a.ts"] },
      { id: "s2", writePaths: ["src/b.ts"] },
    ]);
    expect(disjoint.mode).toBe("fan_out");
    expect(disjoint.independent).toBeTrue();
  });

  test("executes Sequential, Parallel, Verify, Recurse combinators, Blackboard ABSTAIN, streaming pipeline(), code routing, LIFO Saga & budget partition", async () => {
    expect(await sequential(1, [(x) => x + 2, (x) => x * 3])).toBe(9);
    expect(await parallel([1, 2, 3], (x) => x * 10)).toEqual([10, 20, 30]);
    expect(await verify(() => 42, (v) => v === 42)).toBe(42);
    await expect(verify(() => 41, (v) => v === 42)).rejects.toThrow("postcondition");

    const rec = await recurse(0, (n) => n + 1, (n) => n >= 3, 5);
    expect(rec).toEqual({ state: 3, iterations: 3, converged: true });

    const bb = new Blackboard(2, 0.8, 0.5);
    bb.recordEvidence({ deltaLogodds: 1.2, coveredKeys: ["k1"] });
    expect(bb.evaluate().decision).toBe("ABSTAIN"); // coverage 0.5 < 0.8
    bb.recordEvidence({ deltaLogodds: 0.8, coveredKeys: ["k2"] });
    expect(bb.evaluate().decision).toBe("PROCEED");

    const streamed: number[] = [];
    for await (const val of pipeline([1, 2, 3], [(x) => x + 1, (x) => x * 2])) {
      streamed.push(val);
    }
    expect(streamed).toEqual([4, 6, 8]);

    expect(routeNodeContract({ kind: "plan", goal: "x" })).toBe("planner");
    expect(routeNodeContract({ kind: "verify", passed: false, failedTests: ["t1"] })).toBe("builder");
    expect(routeNodeContract({ kind: "verify", passed: true, failedTests: [] })).toBe("release");
    expect(routeNodeContract({ kind: "blocked", reason: "stall" })).toBe("escalate");

    const saga = new SagaStack();
    const order: string[] = [];
    saga.push("step1", () => { order.push("undo1"); });
    saga.push("step2", () => { order.push("undo2"); });
    expect(await saga.rollbackAll()).toEqual(["step2", "step1"]);
    expect(order).toEqual(["undo2", "undo1"]);

    const parts = partitionParentBudget({ maxTurns: 10, maxTokens: 1000, maxUsd: 1.0 }, [1, 2, 1]);
    expect(parts.reduce((s, p) => s + p.maxTurns, 0)).toBeLessThanOrEqual(10);
    expect(parts.reduce((s, p) => s + p.maxTokens, 0)).toBeLessThanOrEqual(1000);
    expect(parts.reduce((s, p) => s + p.maxUsd, 0)).toBeLessThanOrEqual(1.0001);
  });
});