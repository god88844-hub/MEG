// P2-05: HARNESSPRO 5-Profile Budget Classifier, 3-Gear Adaptive Thickness,
// Dual-Cache Model Fusion (240s heartbeat), AgentBaton handoff & EPAM Circuit Breaker.

export type BudgetProfileName = "compact" | "lean" | "standard" | "deep" | "research";
export type BudgetTier = "lite" | "medium" | "heavy";

export interface BudgetAllocation {
  profile: BudgetProfileName;
  upgradedFrom?: BudgetProfileName;
  tier: BudgetTier;
  econ: number;
  strict: number;
  final: number;
  rem: number;
}

const PROFILE_BUDGETS: Record<BudgetProfileName, Omit<BudgetAllocation, "profile" | "upgradedFrom">> = {
  compact: { tier: "lite", econ: 6, strict: 10, final: 12, rem: 2 },
  lean: { tier: "lite", econ: 10, strict: 16, final: 20, rem: 4 },
  standard: { tier: "medium", econ: 18, strict: 28, final: 36, rem: 8 },
  deep: { tier: "heavy", econ: 32, strict: 48, final: 60, rem: 12 },
  research: { tier: "heavy", econ: 40, strict: 60, final: 75, rem: 15 },
};

/**
 * HARNESSPRO 5-profile classifier.
 * Rule: `lean` automatically upgrades to `standard` if classifier confidence < 0.9.
 */
export function classifyBudgetProfile(
  rawProfile: BudgetProfileName,
  confidence: number,
): BudgetAllocation {
  if (rawProfile === "lean" && confidence < 0.9) {
    return {
      profile: "standard",
      upgradedFrom: "lean",
      ...PROFILE_BUDGETS.standard,
    };
  }
  return {
    profile: rawProfile,
    ...PROFILE_BUDGETS[rawProfile],
  };
}

export type HarnessGear = 1 | 2 | 3;

export interface GearConfig {
  gear: HarnessGear;
  label: string;
  preActionLedger: boolean;
  structuredToolsOnly: boolean;
  seltCompiledTools: boolean;
  verifiedDirective: boolean;
  handshakeRequired: boolean;
  thinBashFirst: boolean;
  reuseBase: boolean;
}

/**
 * 3-Gear Adaptive Thickness:
 * - Gear 1 (4B-14B local): pre-action ledger + structured tools + SELT
 * - Gear 2 (20B-35B open): verified directive + handshakes
 * - Gear 3 (Frontier): thin bash-first freedom + Reuse Base
 */
export function selectHarnessGear(paramBillions: number | "frontier"): GearConfig {
  if (paramBillions === "frontier" || paramBillions > 35) {
    return {
      gear: 3,
      label: "Frontier",
      preActionLedger: false,
      structuredToolsOnly: false,
      seltCompiledTools: false,
      verifiedDirective: false,
      handshakeRequired: false,
      thinBashFirst: true,
      reuseBase: true,
    };
  }
  if (paramBillions >= 20) {
    return {
      gear: 2,
      label: "Mid-Open (20B-35B)",
      preActionLedger: false,
      structuredToolsOnly: true,
      seltCompiledTools: false,
      verifiedDirective: true,
      handshakeRequired: true,
      thinBashFirst: false,
      reuseBase: false,
    };
  }
  return {
    gear: 1,
    label: "Small-Local (4B-14B)",
    preActionLedger: true,
    structuredToolsOnly: true,
    seltCompiledTools: true,
    verifiedDirective: true,
    handshakeRequired: true,
    thinBashFirst: false,
    reuseBase: false,
  };
}

export const DUAL_CACHE_HEARTBEAT_MS = 240_000;

export interface DualCacheThreadState {
  role: "head" | "sidekick";
  model: string;
  lastTouchMs: number;
  heartbeatCount: number;
}

/** Dual-Cache Model Fusion coordinator maintaining Head + Sidekick KV cache warmth (240s heartbeat). */
export class DualCacheFusion {
  head: DualCacheThreadState;
  sidekick: DualCacheThreadState;

  constructor(headModel: string, sidekickModel: string, nowMs = Date.now()) {
    this.head = { role: "head", model: headModel, lastTouchMs: nowMs, heartbeatCount: 0 };
    this.sidekick = { role: "sidekick", model: sidekickModel, lastTouchMs: nowMs, heartbeatCount: 0 };
  }

  touch(role: "head" | "sidekick", nowMs = Date.now()): void {
    this[role].lastTouchMs = nowMs;
  }

  /** Return whichever thread(s) need a 240s keep-alive ping to avoid KV-cache eviction. */
  dueHeartbeats(nowMs = Date.now()): ("head" | "sidekick")[] {
    const due: ("head" | "sidekick")[] = [];
    for (const role of ["head", "sidekick"] as const) {
      if (nowMs - this[role].lastTouchMs >= DUAL_CACHE_HEARTBEAT_MS) {
        due.push(role);
      }
    }
    return due;
  }

  recordHeartbeat(role: "head" | "sidekick", nowMs = Date.now()): void {
    this[role].lastTouchMs = nowMs;
    this[role].heartbeatCount += 1;
  }
}

export interface BatonReadinessInput {
  targetModel: string;
  planLocked: boolean;
  evidencePathsExist: boolean;
  openQuestionsZero: boolean;
}

export interface BatonHandoffVerdict {
  ready: boolean;
  missingCriteria: string[];
  transferMode: "logfile" | "in_context";
}

/**
 * AgentBaton 3-criterion readiness handoff:
 * - Hidden-CoT models (e.g. gpt-5 / gpt-sol / o1 / o3) use logfile transfer.
 * - Exposed-context models (Opus / Kimi / Gemini) use in-context transfer.
 */
export function evaluateAgentBaton(input: BatonReadinessInput): BatonHandoffVerdict {
  const missingCriteria: string[] = [];
  if (!input.planLocked) missingCriteria.push("planLocked");
  if (!input.evidencePathsExist) missingCriteria.push("evidencePathsExist");
  if (!input.openQuestionsZero) missingCriteria.push("openQuestionsZero");

  const low = input.targetModel.toLowerCase();
  const hiddenCot =
    low.includes("gpt-sol") || low.includes("gpt-5") || /\bo[134](-|$)/.test(low);

  return {
    ready: missingCriteria.length === 0,
    missingCriteria,
    transferMode: hiddenCot ? "logfile" : "in_context",
  };
}

export interface EpamTurnTelemetry {
  reasoningTokens: number;
  toolCallCount: number;
  inputTokens: number;
  outputTokens: number;
  testProgressMade: boolean;
}

export interface EpamVerdict {
  tripped: boolean;
  action: "continue" | "halt_and_compact";
  reason?: string;
}

/**
 * EPAM Circuit Breaker:
 * Halts/compacts if a single turn burns >8,000 reasoning tokens with 0 tool calls,
 * OR input:output ratio exceeds 100:1 without test progress.
 */
export function checkEpamCircuitBreaker(t: EpamTurnTelemetry): EpamVerdict {
  if (t.reasoningTokens > 8_000 && t.toolCallCount === 0) {
    return {
      tripped: true,
      action: "halt_and_compact",
      reason: `EPAM breaker: ${t.reasoningTokens} reasoning tokens burned with 0 tool calls`,
    };
  }
  const denom = Math.max(1, t.outputTokens);
  const ratio = t.inputTokens / denom;
  if (!t.testProgressMade && ratio > 100) {
    return {
      tripped: true,
      action: "halt_and_compact",
      reason: `EPAM breaker: input:output ratio ${ratio.toFixed(1)}:1 exceeded 100:1 without test progress`,
    };
  }
  return { tripped: false, action: "continue" };
}
// ===========================================================================
// Batch 7.5C: Devin Fusion (2-Thread Brief/Result/Feedback) &
// Antigravity Orchestrator (Bidirectional Single-Trajectory + Default Pair Settings)
// ===========================================================================

import fs from "node:fs";
import path from "node:path";

export type EffortLevel = "low" | "medium" | "high" | "max";
export type PairOrchestrationMode = "devin_fusion" | "antigravity_orchestrator";

export interface DefaultPairSettings {
  mode: PairOrchestrationMode;
  plannerModel: string;
  plannerEffort: EffortLevel;
  executorModel: string;
  executorEffort: EffortLevel;
}

export function saveDefaultPairSettings(workspace: string, settings: DefaultPairSettings): string {
  const file = path.join(workspace, ".meg", "pair-settings.json");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, JSON.stringify(settings, null, 2) + "\n", "utf8");
  return file;
}

export function loadDefaultPairSettings(workspace: string): DefaultPairSettings | null {
  const file = path.join(workspace, ".meg", "pair-settings.json");
  if (!fs.existsSync(file)) return null;
  return JSON.parse(fs.readFileSync(file, "utf8")) as DefaultPairSettings;
}

export interface FusionBrief {
  briefId: string;
  objective: string;
  constraints: string[];
  successCriteria: string[];
  allowedFiles: string[];
}

export interface FusionResult {
  briefId: string;
  editedFiles: string[];
  verificationSummary: string;
  passed: boolean;
}

export interface FusionFeedback {
  briefId: string;
  approved: boolean;
  revisions: string[];
}

/**
 * Devin Fusion (2 Parallel Persistent Agents with Separate Cached Contexts):
 * Lead owns planning & review; Sidekick owns exploration, edits & tests.
 * Communication is restricted strictly to structured Brief / Result / Feedback envelopes
 * so neither thread invalidates the other's KV prompt cache.
 */
export class DevinFusionSession {
  readonly cacheCoordinator: DualCacheFusion;
  readonly leadEffort: EffortLevel;
  readonly sidekickEffort: EffortLevel;
  private leadLog: { kind: "brief" | "feedback"; payload: FusionBrief | FusionFeedback }[] = [];
  private sidekickLog: { kind: "brief" | "result"; payload: FusionBrief | FusionResult }[] = [];
  private seq = 0;

  constructor(settings: Pick<DefaultPairSettings, "plannerModel" | "plannerEffort" | "executorModel" | "executorEffort">, nowMs = Date.now()) {
    this.cacheCoordinator = new DualCacheFusion(settings.plannerModel, settings.executorModel, nowMs);
    this.leadEffort = settings.plannerEffort;
    this.sidekickEffort = settings.executorEffort;
  }

  issueBrief(input: Omit<FusionBrief, "briefId">, nowMs = Date.now()): FusionBrief {
    const brief: FusionBrief = { briefId: `brief-${++this.seq}`, ...input };
    this.leadLog.push({ kind: "brief", payload: brief });
    this.sidekickLog.push({ kind: "brief", payload: brief });
    this.cacheCoordinator.touch("head", nowMs);
    this.cacheCoordinator.touch("sidekick", nowMs);
    return brief;
  }

  submitSidekickResult(result: FusionResult, nowMs = Date.now()): FusionResult {
    this.sidekickLog.push({ kind: "result", payload: result });
    this.cacheCoordinator.touch("sidekick", nowMs);
    return result;
  }

  reviewAndSendFeedback(feedback: FusionFeedback, nowMs = Date.now()): FusionFeedback {
    this.leadLog.push({ kind: "feedback", payload: feedback });
    this.cacheCoordinator.touch("head", nowMs);
    return feedback;
  }

  stats() {
    return {
      leadModel: this.cacheCoordinator.head.model,
      leadEffort: this.leadEffort,
      sidekickModel: this.cacheCoordinator.sidekick.model,
      sidekickEffort: this.sidekickEffort,
      separatePersistentContexts: true as const,
      leadEvents: this.leadLog.length,
      sidekickEvents: this.sidekickLog.length,
    };
  }
}

export type OrchestratorPhase = "explore_plan_readonly" | "execute" | "replan_review" | "complete";

export interface OrchestratorFrictionSignal {
  testsFailed?: boolean;
  consecutiveErrors?: number;
  epamTripped?: boolean;
  scopeExpanded?: boolean;
}

export interface OrchestratorTurnState {
  phase: OrchestratorPhase;
  activeRole: "orchestrator" | "executor";
  activeModel: string;
  activeEffort: EffortLevel;
  readOnly: boolean;
  interventions: number;
  reason: string;
}

/**
 * Antigravity Orchestrator (Dynamic Bidirectional Collaboration in One Trajectory):
 * - Starts in read-only `explore_plan_readonly` with the Orchestrator model.
 * - Hands off to `execute` with the Executor model.
 * - Dynamically re-intervenes back to `replan_review` whenever friction hits
 *   (test failures, >=2 consecutive errors, EPAM circuit breaker, or scope expansion).
 */
export class BidirectionalOrchestrator {
  private phase: OrchestratorPhase = "explore_plan_readonly";
  private interventions = 0;

  constructor(private readonly settings: DefaultPairSettings) {}

  currentState(reason = "initial read-only exploration and planning"): OrchestratorTurnState {
    const isOrch = this.phase !== "execute";
    return {
      phase: this.phase,
      activeRole: isOrch ? "orchestrator" : "executor",
      activeModel: isOrch ? this.settings.plannerModel : this.settings.executorModel,
      activeEffort: isOrch ? this.settings.plannerEffort : this.settings.executorEffort,
      readOnly: this.phase === "explore_plan_readonly" || this.phase === "replan_review",
      interventions: this.interventions,
      reason,
    };
  }

  handoffToExecutor(): OrchestratorTurnState {
    if (this.phase !== "explore_plan_readonly" && this.phase !== "replan_review") {
      throw new Error(`Cannot hand off to executor from phase '${this.phase}'`);
    }
    this.phase = "execute";
    return this.currentState("plan locked; handing off execution to executor model");
  }

  reportExecutorOutcome(signal: OrchestratorFrictionSignal, allVerified = false): OrchestratorTurnState {
    if (this.phase !== "execute") {
      throw new Error(`Executor outcome only valid during 'execute' phase (got '${this.phase}')`);
    }
    const friction =
      Boolean(signal.testsFailed) ||
      (signal.consecutiveErrors ?? 0) >= 2 ||
      Boolean(signal.epamTripped) ||
      Boolean(signal.scopeExpanded);

    if (friction) {
      this.phase = "replan_review";
      this.interventions += 1;
      return this.currentState("friction detected; orchestrator re-intervening for replan/review");
    }
    if (allVerified) {
      this.phase = "complete";
      return this.currentState("executor verified cleanly; trajectory complete");
    }
    return this.currentState("executor continuing cleanly");
  }
}