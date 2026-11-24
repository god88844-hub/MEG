import { describe, expect, test } from "bun:test";
import {
  DUAL_CACHE_HEARTBEAT_MS,
  DualCacheFusion,
  checkEpamCircuitBreaker,
  classifyBudgetProfile,
  evaluateAgentBaton,
  selectHarnessGear,
} from "../src/fusion";

describe("P2-05 Budget Classifier, 3-Gear Thickness, Fusion & EPAM Breaker", () => {
  test("upgrades lean -> standard when confidence < 0.9 and keeps lean at >= 0.9", () => {
    const upgraded = classifyBudgetProfile("lean", 0.72);
    expect(upgraded.profile).toBe("standard");
    expect(upgraded.upgradedFrom).toBe("lean");
    expect(upgraded.tier).toBe("medium");

    const kept = classifyBudgetProfile("lean", 0.95);
    expect(kept.profile).toBe("lean");
    expect(kept.tier).toBe("lite");
  });

  test("selects 3-gear thickness, 240s dual-cache heartbeats, and AgentBaton modes", () => {
    expect(selectHarnessGear(7).preActionLedger).toBeTrue();
    expect(selectHarnessGear(7).seltCompiledTools).toBeTrue();
    expect(selectHarnessGear(32).verifiedDirective).toBeTrue();
    expect(selectHarnessGear("frontier").thinBashFirst).toBeTrue();
    expect(selectHarnessGear("frontier").reuseBase).toBeTrue();

    const fusion = new DualCacheFusion("claude-opus", "qwen2.5-coder:14b", 1000);
    expect(fusion.dueHeartbeats(1000 + 10_000)).toEqual([]);
    expect(fusion.dueHeartbeats(1000 + DUAL_CACHE_HEARTBEAT_MS)).toEqual(["head", "sidekick"]);

    expect(
      evaluateAgentBaton({
        targetModel: "gpt-sol",
        planLocked: true,
        evidencePathsExist: true,
        openQuestionsZero: true,
      }).transferMode,
    ).toBe("logfile");

    const opusBaton = evaluateAgentBaton({
      targetModel: "claude-opus-4",
      planLocked: true,
      evidencePathsExist: false,
      openQuestionsZero: true,
    });
    expect(opusBaton.ready).toBeFalse();
    expect(opusBaton.transferMode).toBe("in_context");
  });

  test("EPAM circuit breaker trips on >8k reasoning tokens with 0 tool calls or >100:1 ratio without progress", () => {
    expect(
      checkEpamCircuitBreaker({
        reasoningTokens: 8_500,
        toolCallCount: 0,
        inputTokens: 10_000,
        outputTokens: 500,
        testProgressMade: false,
      }).tripped,
    ).toBeTrue();

    expect(
      checkEpamCircuitBreaker({
        reasoningTokens: 200,
        toolCallCount: 2,
        inputTokens: 50_000,
        outputTokens: 100,
        testProgressMade: false,
      }).tripped,
    ).toBeTrue();

    expect(
      checkEpamCircuitBreaker({
        reasoningTokens: 200,
        toolCallCount: 2,
        inputTokens: 50_000,
        outputTokens: 100,
        testProgressMade: true,
      }).tripped,
    ).toBeFalse();
  });
});