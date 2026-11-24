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