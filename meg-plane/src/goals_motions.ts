// P5-04: Goal & Motion Engineering:
// - Dual-Cadence handoff (STATE.md loops -> GOAL.md + goal-budget.md + quiet update_goal telemetry + /goal status|pause|resume|clear)
// - Anti-Goal-Gaming floor invariants (total_test_assertions >= baseline, 0 protected files deleted)
// - Background Entity Context Gatherers + horizontal Motion Agents (Homogeneous Batch Review Queue)
// - Isolated scheduled routine runner with 0-token pre-run delta check (routines/state/<name>.txt) in dontAsk mode

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { GoalSpec } from "./types";

export interface GoalBudgetState {
  maxUsd: number;
  spentUsd: number;
  maxRuns: number;
  runsUsed: number;
  updatedAt: number;
}

export interface GoalTelemetryUpdate {
  goalId: string;
  deltaProgress: string;
  spentUsdDelta: number;
  quiet: true;
  timestamp: number;
}

export interface GoalCommandResult {
  command: string;
  ok: boolean;
  status: GoalSpec["status"] | "none";
  message: string;
  budget?: GoalBudgetState;
}

export class GoalManager {
  private readonly goalFile: string;
  private readonly budgetFile: string;
  private readonly telemetryFile: string;
  private goal: GoalSpec | null = null;
  private budget: GoalBudgetState | null = null;

  constructor(private readonly workspace: string) {
    this.goalFile = path.join(workspace, "GOAL.md");
    this.budgetFile = path.join(workspace, "goal-budget.md");
    this.telemetryFile = path.join(workspace, ".meg", "goal-telemetry.jsonl");
  }

  /** Dual-Cadence handoff: elevate a verified STATE.md loop into GOAL.md + goal-budget.md. */
  handoffFromState(
    spec: Omit<GoalSpec, "status">,
    budget: { maxUsd: number; maxRuns: number },
  ): { goalFile: string; budgetFile: string; spec: GoalSpec } {
    this.goal = { ...spec, status: "active" };
    this.budget = {
      maxUsd: budget.maxUsd,
      spentUsd: 0,
      maxRuns: budget.maxRuns,
      runsUsed: 0,
      updatedAt: Date.now(),
    };
    this.persist();
    return { goalFile: this.goalFile, budgetFile: this.budgetFile, spec: { ...this.goal } };
  }

  private persist(): void {
    if (!this.goal || !this.budget) return;
    const goalMd = [
      `# GOAL: ${this.goal.title}`,
      ``,
      `- id: ${this.goal.id}`,
      `- status: ${this.goal.status}`,
      `- budgetUsd: ${this.goal.budgetUsd}`,
      ``,
      `## Objective`,
      this.goal.objective,
      ``,
      `## Metrics`,
      ...this.goal.metrics.map((m) => `- ${m}`),
      ``,
      `## Floor Invariants`,
      ...this.goal.floorInvariants.map((f) => `- ${f}`),
      ``,
    ].join("\n");
    fs.writeFileSync(this.goalFile, goalMd, "utf8");

    const budgetMd = [
      `# Goal Budget`,
      ``,
      `- maxUsd: ${this.budget.maxUsd.toFixed(2)}`,
      `- spentUsd: ${this.budget.spentUsd.toFixed(2)}`,
      `- maxRuns: ${this.budget.maxRuns}`,
      `- runsUsed: ${this.budget.runsUsed}`,
      `- updatedAt: ${this.budget.updatedAt}`,
      ``,
    ].join("\n");
    fs.writeFileSync(this.budgetFile, budgetMd, "utf8");
  }

  /** Quiet `update_goal` telemetry update (no noisy chat spam; checks budget ceiling). */
  updateGoal(deltaProgress: string, spentUsdDelta: number): GoalTelemetryUpdate {
    if (!this.goal || !this.budget) {
      throw new Error("No active goal initialized");
    }
    if (this.goal.status !== "active") {
      throw new Error(`Goal '${this.goal.id}' is not active (status=${this.goal.status})`);
    }
    this.budget.spentUsd = Math.round((this.budget.spentUsd + spentUsdDelta) * 10_000) / 10_000;
    this.budget.runsUsed += 1;
    this.budget.updatedAt = Date.now();
    if (this.budget.spentUsd > this.budget.maxUsd || this.budget.runsUsed > this.budget.maxRuns) {
      this.goal.status = "paused";
    }
    this.persist();

    const ev: GoalTelemetryUpdate = {
      goalId: this.goal.id,
      deltaProgress,
      spentUsdDelta,
      quiet: true,
      timestamp: Date.now(),
    };
    fs.mkdirSync(path.dirname(this.telemetryFile), { recursive: true });
    fs.appendFileSync(this.telemetryFile, JSON.stringify(ev) + "\n", "utf8");
    return ev;
  }

  /** Handle `/goal status | pause | resume | clear`. */
  handleCommand(raw: string): GoalCommandResult {
    const parts = raw.trim().split(/\s+/);
    const sub = (parts[1] ?? "status").toLowerCase();
    if (!this.goal || !this.budget) {
      return { command: raw, ok: false, status: "none", message: "No active goal configured" };
    }
    switch (sub) {
      case "status":
        return {
          command: "/goal status",
          ok: true,
          status: this.goal.status,
          message: `Goal ${this.goal.id} (${this.goal.status}): $${this.budget.spentUsd.toFixed(2)}/$${this.budget.maxUsd.toFixed(2)}, runs ${this.budget.runsUsed}/${this.budget.maxRuns}`,
          budget: { ...this.budget },
        };
      case "pause":
        this.goal.status = "paused";
        this.persist();
        return { command: "/goal pause", ok: true, status: "paused", message: `Goal ${this.goal.id} paused`, budget: { ...this.budget } };
      case "resume":
        this.goal.status = "active";
        this.persist();
        return { command: "/goal resume", ok: true, status: "active", message: `Goal ${this.goal.id} resumed`, budget: { ...this.budget } };
      case "clear":
        this.goal.status = "cleared";
        this.persist();
        return { command: "/goal clear", ok: true, status: "cleared", message: `Goal ${this.goal.id} cleared`, budget: { ...this.budget } };
      default:
        return { command: raw, ok: false, status: this.goal.status, message: `Unknown /goal action '${sub}' (use status|pause|resume|clear)` };
    }
  }
}

export interface FloorInvariantCheckInput {
  baselineAssertions: number;
  currentAssertions: number;
  deletedFiles: readonly string[];
  protectedPrefixes?: readonly string[];
}

export interface FloorInvariantVerdict {
  ok: boolean;
  violations: string[];
}

/**
 * Anti-Goal-Gaming floor invariants:
 * 1. `total_test_assertions >= baseline`
 * 2. `0 protected files deleted`
 */
export function checkAntiGoalGamingInvariants(input: FloorInvariantCheckInput): FloorInvariantVerdict {
  const violations: string[] = [];
  const protectedPrefixes = input.protectedPrefixes ?? ["tests/", "meg-plane/test/", "migrations/", "scripts/check."];

  if (input.currentAssertions < input.baselineAssertions) {
    violations.push(
      `ASSERTION_REGRESSION: total_test_assertions (${input.currentAssertions}) dropped below baseline (${input.baselineAssertions})`,
    );
  }
  for (const file of input.deletedFiles) {
    const norm = file.replace(/\\/g, "/");
    if (protectedPrefixes.some((p) => norm.includes(p))) {
      violations.push(`PROTECTED_FILE_DELETED: '${norm}'`);
    }
  }
  return { ok: violations.length === 0, violations };
}

export interface EntityContextCard {
  entityId: string;
  entityType: string;
  facts: string[];
  gatheredAt: number;
}

export interface MotionDraftItem {
  draftId: string;
  entityId: string;
  motionType: string; // e.g. "pr_description", "release_note", "outreach"
  content: string;
  tasteTags?: string[];
}

export interface HomogeneousReviewBatch {
  motionType: string;
  items: { draft: MotionDraftItem; entityContext?: EntityContextCard }[];
  sharedTasteChecklist: string[];
}

/**
 * Background Entity Context Gatherer + horizontal Motion Agent batcher:
 * Groups pending drafts by `motionType` into a Homogeneous Batch Review Queue
 * so human taste edits transfer across all entities in one pass.
 */
export class MotionBatchCoordinator {
  private entities = new Map<string, EntityContextCard>();
  private pendingDrafts: MotionDraftItem[] = [];

  gatherEntityContext(entityId: string, entityType: string, facts: readonly string[]): EntityContextCard {
    const card: EntityContextCard = {
      entityId,
      entityType,
      facts: [...facts],
      gatheredAt: Date.now(),
    };
    this.entities.set(entityId, card);
    return card;
  }

  enqueueDraft(draft: MotionDraftItem): void {
    this.pendingDrafts.push({ ...draft });
  }

  buildHomogeneousReviewQueues(): HomogeneousReviewBatch[] {
    const grouped = new Map<string, MotionDraftItem[]>();
    for (const draft of this.pendingDrafts) {
      const list = grouped.get(draft.motionType) ?? [];
      list.push(draft);
      grouped.set(draft.motionType, list);
    }

    const batches: HomogeneousReviewBatch[] = [];
    for (const [motionType, drafts] of [...grouped.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      const sharedTasteChecklist = [
        ...new Set(drafts.flatMap((d) => d.tasteTags ?? [])),
      ].sort();
      batches.push({
        motionType,
        items: drafts.map((draft) => ({
          draft,
          entityContext: this.entities.get(draft.entityId),
        })),
        sharedTasteChecklist,
      });
    }
    return batches;
  }
}

export interface RoutineExecutionResult {
  routine: string;
  mode: "dontAsk";
  ranModel: boolean;
  tokensUsed: number;
  stateHash: string;
  reason: string;
}

/**
 * Isolated scheduled routine runner with 0-token pre-run delta check (`routines/state/<name>.txt`)
 * operating strictly in `dontAsk` mode.
 */
export async function runScheduledRoutine(
  workspace: string,
  routineName: string,
  currentInputSnapshot: string,
  executeWork: () => Promise<{ tokensUsed: number; summary: string }>,
): Promise<RoutineExecutionResult> {
  const safeName = routineName.replace(/[^a-zA-Z0-9_-]/g, "_");
  const stateFile = path.join(workspace, "routines", "state", `${safeName}.txt`);
  const nextHash = createHash("sha256").update(currentInputSnapshot).digest("hex");

  if (fs.existsSync(stateFile)) {
    const prevHash = fs.readFileSync(stateFile, "utf8").trim();
    if (prevHash === nextHash) {
      return {
        routine: safeName,
        mode: "dontAsk",
        ranModel: false,
        tokensUsed: 0,
        stateHash: nextHash,
        reason: "0-token pre-run delta check: state unchanged since last run",
      };
    }
  }

  const work = await executeWork();
  fs.mkdirSync(path.dirname(stateFile), { recursive: true });
  fs.writeFileSync(stateFile, nextHash + "\n", "utf8");
  return {
    routine: safeName,
    mode: "dontAsk",
    ranModel: true,
    tokensUsed: work.tokensUsed,
    stateHash: nextHash,
    reason: work.summary,
  };
}