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
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  BidirectionalOrchestrator,
  DevinFusionSession,
  loadDefaultPairSettings,
  saveDefaultPairSettings,
} from "../src/fusion";

describe("Batch 7.5C Devin Fusion & Antigravity Bidirectional Orchestrator", () => {
  test("DevinFusionSession maintains 2 separate persistent contexts communicating via Brief/Result/Feedback & persists pair settings", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-pair-"));
    try {
      const settings = {
        mode: "devin_fusion" as const,
        plannerModel: "claude-opus-4",
        plannerEffort: "high" as const,
        executorModel: "qwen2.5-coder:32b",
        executorEffort: "medium" as const,
      };
      saveDefaultPairSettings(ws, settings);
      expect(loadDefaultPairSettings(ws)).toEqual(settings);

      const session = new DevinFusionSession(settings, 1000);
      const brief = session.issueBrief(
        {
          objective: "Add retry backoff",
          constraints: ["keep zero deps"],
          successCriteria: ["429 test passes"],
          allowedFiles: ["src/client.ts"],
        },
        2000,
      );
      session.submitSidekickResult(
        { briefId: brief.briefId, editedFiles: ["src/client.ts"], verificationSummary: "12 pass", passed: true },
        3000,
      );
      session.reviewAndSendFeedback({ briefId: brief.briefId, approved: true, revisions: [] }, 4000);
      const st = session.stats();
      expect(st.separatePersistentContexts).toBeTrue();
      expect(st.leadEvents).toBe(2);
      expect(st.sidekickEvents).toBe(2);
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("BidirectionalOrchestrator runs read-only explore/plan -> execute -> friction re-intervention -> complete", () => {
    const orch = new BidirectionalOrchestrator({
      mode: "antigravity_orchestrator",
      plannerModel: "gemini-4-neon",
      plannerEffort: "high",
      executorModel: "gemini-4-hydrogen",
      executorEffort: "high",
    });

    const s0 = orch.currentState();
    expect(s0.phase).toBe("explore_plan_readonly");
    expect(s0.readOnly).toBeTrue();
    expect(s0.activeModel).toBe("gemini-4-neon");

    const s1 = orch.handoffToExecutor();
    expect(s1.phase).toBe("execute");
    expect(s1.readOnly).toBeFalse();
    expect(s1.activeModel).toBe("gemini-4-hydrogen");

    // Friction triggers dynamic orchestrator re-intervention
    const s2 = orch.reportExecutorOutcome({ testsFailed: true });
    expect(s2.phase).toBe("replan_review");
    expect(s2.readOnly).toBeTrue();
    expect(s2.interventions).toBe(1);

    // Re-handoff -> verified completion
    orch.handoffToExecutor();
    const s3 = orch.reportExecutorOutcome({}, true);
    expect(s3.phase).toBe("complete");
  });
});