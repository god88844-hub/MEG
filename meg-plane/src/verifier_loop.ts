// P4-01: Verified Work Loop (Loop 2) & Stall/EFC Guards:
// - Pre-run check_cannot_fail gate (Green -> Red -> Green)
// - <3,000-char FAILED / EXPECTED / SAW / file:line feedback formatter
// - Path+digit-masked fingerprint phi(e) & .loop/state.json tried ledger
// - 3-pattern loop detector (Repetition, Stagnation, A->B->A Cycling)
// - EFC (I * V * R * M) monitor, MAX_SAME = 2 stall exit -> blocked/<id>
// - Karpathy Clean-Slate Replay (>4 messy retries -> side-git reset -> 1 fresh pass)
// - Concentric 4-Layer Diagnostic Router (Harness -> Loop -> Context -> Prompt)
// - DeclineCraft calibrated abstention & Adaptive Replay Refinement

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const MAX_FEEDBACK_CHARS = 3_000;
export const MAX_SAME_FINGERPRINT = 2;
export const CLEAN_SLATE_RETRY_THRESHOLD = 4;

export interface CheckCannotFailResult {
  valid: boolean;
  baselineGreen: boolean;
  mutatedRed: boolean;
  restoredGreen: boolean;
  reason?: string;
}

/** Pre-run check_cannot_fail gate: proves the check suite actually catches a fault (Green -> Red -> Green). */
export async function runCheckCannotFailGate(probes: {
  runCheck: () => Promise<boolean> | boolean;
  plantFault: () => Promise<void> | void;
  revertFault: () => Promise<void> | void;
}): Promise<CheckCannotFailResult> {
  const baselineGreen = await probes.runCheck();
  if (!baselineGreen) {
    return {
      valid: false,
      baselineGreen: false,
      mutatedRed: false,
      restoredGreen: false,
      reason: "Baseline check failed before fault injection",
    };
  }
  let mutatedRed = false;
  try {
    await probes.plantFault();
    mutatedRed = !(await probes.runCheck());
  } finally {
    await probes.revertFault();
  }
  const restoredGreen = await probes.runCheck();
  const valid = baselineGreen && mutatedRed && restoredGreen;
  return {
    valid,
    baselineGreen,
    mutatedRed,
    restoredGreen,
    reason: !mutatedRed
      ? "CHECK_CANNOT_FAIL: suite still passed with synthetic fault planted"
      : !restoredGreen
        ? "Post-revert check failed"
        : undefined,
  };
}

export interface StructuredFailureInput {
  failed: string;
  expected: string;
  saw: string;
  location: string; // file:line
}

/** Format failure feedback into <3,000-char FAILED / EXPECTED / SAW / file:line card. */
export function formatStructuredFeedback(input: StructuredFailureInput): string {
  const card = [
    `FAILED: ${input.failed.trim()}`,
    `EXPECTED: ${input.expected.trim()}`,
    `SAW: ${input.saw.trim()}`,
    `LOCATION: ${input.location.trim()}`,
  ].join("\n");
  if (card.length <= MAX_FEEDBACK_CHARS) return card;
  return card.slice(0, MAX_FEEDBACK_CHARS - 20) + "\n...[truncated]";
}

/** Path+digit-masked fingerprint phi(e) so line numbers/timestamps/paths collapse to one canonical error signature. */
export function fingerprintError(rawError: string): string {
  const masked = rawError
    .replace(/[A-Za-z]:\\[^\s:'"]+/g, "<PATH>")
    .replace(/(?:^|[\s(])\/[\w./-]+/g, " <PATH>")
    .replace(/\b0x[0-9a-fA-F]+\b/g, "<HEX>")
    .replace(/\d+/g, "<N>")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
  return createHash("sha256").update(masked).digest("hex").slice(0, 16);
}

export type LoopPattern = "none" | "Repetition" | "Stagnation" | "A->B->A Cycling";

export interface TriedAttempt {
  attempt: number;
  fingerprint: string;
  hypothesis: string;
  diffHash: string;
  passedTests: number;
  timestamp: number;
}

export interface LoopState {
  taskId: string;
  tried: TriedAttempt[];
  parkedBranch: string | null;
  cleanSlateTriggered: boolean;
}

export interface EfcMetrics {
  I: number; // Information gain [0,1]
  V: number; // Validity / test signal [0,1]
  R: number; // Relevance to goal [0,1]
  M: number; // Momentum (non-stall progress) [0,1]
}

export function computeEfcScore(m: EfcMetrics): number {
  const clamp = (x: number) => Math.max(0, Math.min(1, x));
  return clamp(m.I) * clamp(m.V) * clamp(m.R) * clamp(m.M);
}

export function detectLoopPattern(tried: readonly TriedAttempt[]): LoopPattern {
  const n = tried.length;
  if (n < 2) return "none";

  // 1. Repetition: last 2 attempts produced the exact same fingerprint.
  if (tried[n - 1]!.fingerprint === tried[n - 2]!.fingerprint) {
    return "Repetition";
  }

  // 2. A->B->A Cycling: alternating fingerprints or diff hashes (A -> B -> A).
  if (
    n >= 3 &&
    tried[n - 1]!.fingerprint === tried[n - 3]!.fingerprint &&
    tried[n - 1]!.fingerprint !== tried[n - 2]!.fingerprint
  ) {
    return "A->B->A Cycling";
  }

  // 3. Stagnation: 3 consecutive attempts with zero increase in passedTests.
  if (
    n >= 3 &&
    tried[n - 1]!.passedTests <= tried[n - 2]!.passedTests &&
    tried[n - 2]!.passedTests <= tried[n - 3]!.passedTests
  ) {
    return "Stagnation";
  }

  return "none";
}

export interface LoopStepDecision {
  action: "continue" | "park_blocked" | "clean_slate_replay";
  pattern: LoopPattern;
  sameFingerprintCount: number;
  efcScore: number;
  parkedBranch?: string;
  reason: string;
}

/** Persistent `.loop/state.json` manager for Loop 2 verification engineering. */
export class VerifierLoopLedger {
  private readonly stateFile: string;
  private state: LoopState;

  constructor(workspace: string, taskId: string) {
    this.stateFile = path.join(workspace, ".loop", "state.json");
    if (fs.existsSync(this.stateFile)) {
      const loaded = JSON.parse(fs.readFileSync(this.stateFile, "utf8")) as LoopState;
      this.state = loaded.taskId === taskId ? loaded : { taskId, tried: [], parkedBranch: null, cleanSlateTriggered: false };
    } else {
      this.state = { taskId, tried: [], parkedBranch: null, cleanSlateTriggered: false };
    }
  }

  private save(): void {
    fs.mkdirSync(path.dirname(this.stateFile), { recursive: true });
    fs.writeFileSync(this.stateFile, JSON.stringify(this.state, null, 2), "utf8");
  }

  snapshot(): LoopState {
    return {
      ...this.state,
      tried: this.state.tried.map((t) => ({ ...t })),
    };
  }

  recordAttempt(input: {
    rawError: string;
    hypothesis: string;
    diffHash: string;
    passedTests: number;
    efc: EfcMetrics;
  }): LoopStepDecision {
    const fingerprint = fingerprintError(input.rawError);
    this.state.tried.push({
      attempt: this.state.tried.length + 1,
      fingerprint,
      hypothesis: input.hypothesis,
      diffHash: input.diffHash,
      passedTests: input.passedTests,
      timestamp: Date.now(),
    });

    const pattern = detectLoopPattern(this.state.tried);
    const sameFingerprintCount = this.state.tried.filter((t) => t.fingerprint === fingerprint).length;
    const efcScore = computeEfcScore(input.efc);

    // Karpathy Clean-Slate Replay after >4 messy retries (before parking forever).
    if (this.state.tried.length > CLEAN_SLATE_RETRY_THRESHOLD && !this.state.cleanSlateTriggered) {
      this.state.cleanSlateTriggered = true;
      this.save();
      return {
        action: "clean_slate_replay",
        pattern,
        sameFingerprintCount,
        efcScore,
        reason: `Exceeded ${CLEAN_SLATE_RETRY_THRESHOLD} messy retries: reset via side-git and re-implement cleanly in 1 fresh pass`,
      };
    }

    // MAX_SAME = 2 stall exit parking on blocked/<id>.
    if (sameFingerprintCount >= MAX_SAME_FINGERPRINT || pattern === "A->B->A Cycling") {
      const parkedBranch = `blocked/${this.state.taskId}`;
      this.state.parkedBranch = parkedBranch;
      this.save();
      return {
        action: "park_blocked",
        pattern,
        sameFingerprintCount,
        efcScore,
        parkedBranch,
        reason: `Stall exit (${pattern}, same=${sameFingerprintCount}/${MAX_SAME_FINGERPRINT}): parked on ${parkedBranch}`,
      };
    }

    this.save();
    return {
      action: "continue",
      pattern,
      sameFingerprintCount,
      efcScore,
      reason: efcScore < 0.15 ? "Low EFC warning; narrow hypothesis" : "Proceed to next verified iteration",
    };
  }
}

export type DiagnosticLayer = "Harness" | "Loop" | "Context" | "Prompt";

/** Concentric 4-Layer Diagnostic Router (`Harness -> Loop -> Context -> Prompt`). */
export function routeDiagnosticLayer(signals: {
  sandboxOrToolInfraFailed?: boolean;
  loopPattern?: LoopPattern;
  missingOrStaleContext?: boolean;
  ambiguousInstruction?: boolean;
}): DiagnosticLayer {
  if (signals.sandboxOrToolInfraFailed) return "Harness";
  if (signals.loopPattern && signals.loopPattern !== "none") return "Loop";
  if (signals.missingOrStaleContext) return "Context";
  return "Prompt";
}

export interface AbstentionVerdict {
  abstain: boolean;
  confidence: number;
  reason: string;
}

/** DeclineCraft calibrated abstention: abstain cleanly rather than hallucinating under low evidence. */
export function evaluateDeclineCraft(input: {
  evidenceCoverage: number; // [0, 1]
  contradictorySignals: boolean;
  missingRequiredPermissions: boolean;
  threshold?: number;
}): AbstentionVerdict {
  const threshold = input.threshold ?? 0.6;
  if (input.missingRequiredPermissions) {
    return { abstain: true, confidence: 0, reason: "Missing required permissions/credentials; abstaining cleanly." };
  }
  const penalty = input.contradictorySignals ? 0.35 : 0;
  const confidence = Math.max(0, Math.min(1, input.evidenceCoverage - penalty));
  if (confidence < threshold) {
    return {
      abstain: true,
      confidence,
      reason: `Calibrated confidence ${confidence.toFixed(2)} < ${threshold.toFixed(2)}; abstaining instead of guessing.`,
    };
  }
  return { abstain: false, confidence, reason: "Sufficient calibrated confidence to proceed." };
}

/** Adaptive Replay Refinement: distills failed attempts into a compact clean-slate prompt brief. */
export function buildAdaptiveReplayBrief(tried: readonly TriedAttempt[]): string {
  const uniqueHypotheses = [...new Set(tried.map((t) => t.hypothesis))];
  const uniqueFingerprints = [...new Set(tried.map((t) => t.fingerprint))];
  return [
    "## Adaptive Replay Refinement (Clean-Slate Pass)",
    `- Prior failed attempts discarded: ${tried.length} (${uniqueFingerprints.length} distinct error signature(s))`,
    "- Do NOT repeat these failed hypotheses:",
    ...uniqueHypotheses.map((h) => `  - ${h}`),
    "- Re-implement cleanly from the pre-edit side-git snapshot in 1 focused pass.",
  ].join("\n");
}