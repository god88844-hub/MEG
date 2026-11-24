import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  GoalManager,
  MotionBatchCoordinator,
  checkAntiGoalGamingInvariants,
  runScheduledRoutine,
} from "../src/goals_motions";

describe("P5-04 Goal & Motion Engineering", () => {
  test("Dual-Cadence handoff writes GOAL.md + goal-budget.md, supports quiet update_goal & /goal status|pause|resume|clear", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p504-goal-"));
    try {
      const gm = new GoalManager(ws);
      const { goalFile, budgetFile } = gm.handoffFromState(
        {
          id: "G-1",
          title: "Improve Offline Reliability",
          objective: "Keep Pass^4 >= 95%",
          metrics: ["Pass^4 >= 0.95"],
          floorInvariants: ["total_test_assertions >= baseline"],
          budgetUsd: 5.0,
        },
        { maxUsd: 5.0, maxRuns: 10 },
      );
      expect(fs.existsSync(goalFile)).toBeTrue();
      expect(fs.existsSync(budgetFile)).toBeTrue();

      const tel = gm.updateGoal("completed 1 eval batch", 1.25);
      expect(tel.quiet).toBeTrue();
      expect(gm.handleCommand("/goal status").message).toContain("$1.25/$5.00");
      expect(gm.handleCommand("/goal pause").status).toBe("paused");
      expect(gm.handleCommand("/goal resume").status).toBe("active");
      expect(gm.handleCommand("/goal clear").status).toBe("cleared");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("Anti-Goal-Gaming floor invariants, Motion Batch Review Queue & 0-token delta scheduled routines", async () => {
    const bad = checkAntiGoalGamingInvariants({
      baselineAssertions: 100,
      currentAssertions: 85,
      deletedFiles: ["meg-plane/test/plane.test.ts"],
    });
    expect(bad.ok).toBeFalse();
    expect(bad.violations.some((v) => v.includes("ASSERTION_REGRESSION"))).toBeTrue();
    expect(bad.violations.some((v) => v.includes("PROTECTED_FILE_DELETED"))).toBeTrue();

    expect(
      checkAntiGoalGamingInvariants({
        baselineAssertions: 100,
        currentAssertions: 105,
        deletedFiles: ["tmp/scratch.txt"],
      }).ok,
    ).toBeTrue();

    const motions = new MotionBatchCoordinator();
    motions.gatherEntityContext("repo-a", "service", ["uses bun"]);
    motions.gatherEntityContext("repo-b", "service", ["uses rust"]);
    motions.enqueueDraft({ draftId: "d1", entityId: "repo-a", motionType: "release_note", content: "v1.0", tasteTags: ["concise"] });
    motions.enqueueDraft({ draftId: "d2", entityId: "repo-b", motionType: "release_note", content: "v1.1", tasteTags: ["metrics-first"] });
    const batches = motions.buildHomogeneousReviewQueues();
    expect(batches).toHaveLength(1);
    expect(batches[0]?.items).toHaveLength(2);
    expect(batches[0]?.sharedTasteChecklist).toEqual(["concise", "metrics-first"]);

    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p504-routine-"));
    try {
      let modelCalls = 0;
      const first = await runScheduledRoutine(ws, "morning-brief", "snapshot-v1", async () => {
        modelCalls++;
        return { tokensUsed: 400, summary: "generated brief" };
      });
      expect(first.ranModel).toBeTrue();
      expect(first.mode).toBe("dontAsk");

      // Unchanged state -> 0-token pre-run skip!
      const second = await runScheduledRoutine(ws, "morning-brief", "snapshot-v1", async () => {
        modelCalls++;
        return { tokensUsed: 400, summary: "should not run" };
      });
      expect(second.ranModel).toBeFalse();
      expect(second.tokensUsed).toBe(0);
      expect(modelCalls).toBe(1);
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});