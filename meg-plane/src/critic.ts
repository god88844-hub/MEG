// P4-04: L1 After-Edit Reminders & Cold Cross-Model L3 Critic:
// - Disables wasteful L2 self-recheck prompts (L2_SELF_RECHECK_ENABLED = false)
// - Emits L1 after-edit reminders (strictly capped at 4/run)
// - Assigns a cold cross-model critic (different model family, >= writer capability)
//   in a fresh context seeing ONLY Contract + git diff + test logs
// - Supports Claim-guided: changes, Width = 1..4 sequential critics before 1 revision,
//   conclusion-blind mode, /tmp throwaway probes only, PROXY / SHARED MISTAKE bans,
//   and non-mutation enforcement.

import type { ModelFamily } from "./router";

export const L2_SELF_RECHECK_ENABLED = false;
export const MAX_L1_REMINDERS_PER_RUN = 4;
export const MIN_CRITIC_WIDTH = 1;
export const MAX_CRITIC_WIDTH = 4;

/** L1 after-edit reminder emitter (strictly capped at 4 reminders per run; L2 self-recheck disabled). */
export class L1AfterEditReminder {
  private emitted = 0;

  get count(): number {
    return this.emitted;
  }

  emit(editedFile: string, checkHint: string): string | null {
    if (this.emitted >= MAX_L1_REMINDERS_PER_RUN) {
      return null;
    }
    this.emitted += 1;
    return `[L1 REMINDER ${this.emitted}/${MAX_L1_REMINDERS_PER_RUN}] Edited '${editedFile}'. Verify: ${checkHint}`;
  }
}

export interface CriticModelCandidate {
  id: string;
  family: ModelFamily;
  capabilityTier: number; // e.g., 1=small, 2=mid, 3=frontier
}

/**
 * Harness cross-model assignment:
 * Must pick a DIFFERENT model family with capabilityTier >= writer.capabilityTier.
 */
export function assignColdCrossModelCritic(
  writer: CriticModelCandidate,
  pool: readonly CriticModelCandidate[],
): CriticModelCandidate {
  const eligible = pool
    .filter((c) => c.family !== writer.family && c.capabilityTier >= writer.capabilityTier)
    .sort((a, b) => b.capabilityTier - a.capabilityTier || a.id.localeCompare(b.id));
  if (eligible.length === 0) {
    throw new Error(
      `CRITIC_ASSIGNMENT_FAILED: no cross-family model available with capabilityTier >= ${writer.capabilityTier} (writer family: ${writer.family})`,
    );
  }
  return eligible[0]!;
}

export interface CriticContextInput {
  contract: string;
  gitDiff: string;
  testLogs: string;
  makerConclusion?: string; // Stripped in conclusion-blind mode!
  makerChatTranscript?: string; // Never passed to cold critic!
  conclusionBlind?: boolean;
  claimGuidedChanges?: readonly string[];
}

export interface ColdCriticPacket {
  freshContext: true;
  nonMutating: true;
  allowedProbeDir: "/tmp";
  conclusionBlind: boolean;
  prompt: string;
}

/**
 * Build an isolated fresh-context packet for the cold L3 critic seeing ONLY
 * Contract + git diff + test logs (never maker chat history; strips maker conclusion when conclusionBlind).
 */
export function buildColdCriticPacket(input: CriticContextInput): ColdCriticPacket {
  const conclusionBlind = input.conclusionBlind !== false;
  const sections: string[] = [
    "## Role\nYou are a cold cross-model L3 Critic. Non-mutation is strictly enforced. Throwaway probes may write ONLY under /tmp.",
    "## Banned Verdicts\nReject PROXY metrics and SHARED MISTAKE rationalizations.",
    `## Contract\n${input.contract.trim()}`,
    `## Git Diff\n${input.gitDiff.trim()}`,
    `## Test Logs\n${input.testLogs.trim()}`,
  ];
  if (input.claimGuidedChanges && input.claimGuidedChanges.length > 0) {
    sections.push(
      `## Claim-guided: changes\n${input.claimGuidedChanges.map((c, i) => `${i + 1}. ${c}`).join("\n")}`,
    );
  }
  if (!conclusionBlind && input.makerConclusion) {
    sections.push(`## Maker Note\n${input.makerConclusion.trim()}`);
  }
  return {
    freshContext: true,
    nonMutating: true,
    allowedProbeDir: "/tmp",
    conclusionBlind,
    prompt: sections.join("\n\n"),
  };
}

/** Enforce critic non-mutation and `/tmp`-only throwaway probes. */
export function enforceCriticToolPolicy(
  tool: string,
  targetPath?: string,
): { allowed: boolean; reason?: string } {
  if (tool === "edit_file") {
    return { allowed: false, reason: "CRITIC_NON_MUTATION: critic cannot edit workspace files" };
  }
  if (tool === "write_file") {
    const norm = (targetPath ?? "").replace(/\\/g, "/");
    if (!norm.startsWith("/tmp/") && !norm.startsWith("tmp/")) {
      return {
        allowed: false,
        reason: `CRITIC_PROBE_POLICY: critic writes are restricted to /tmp throwaway probes (got '${norm}')`,
      };
    }
  }
  return { allowed: true };
}

export interface CriticFinding {
  criticId: string;
  approved: boolean;
  issues: string[];
  rationale: string;
}

export interface CriticPanelVerdict {
  width: number;
  approved: boolean;
  revisionsAllowed: 1;
  bannedPatternHits: string[];
  mergedIssues: string[];
}

const BANNED_CRITIC_PATTERNS: readonly { code: string; re: RegExp }[] = [
  { code: "PROXY", re: /\bPROXY\b|proxy metric|approximates? the real check/i },
  { code: "SHARED_MISTAKE", re: /\bSHARED MISTAKE\b|same mistake as (the )?(maker|writer)|acceptable shared error/i },
];

/**
 * Width = 1..4 sequential critics before 1 consolidated revision.
 * Rejects any critic rationale containing banned PROXY / SHARED MISTAKE patterns.
 */
export function aggregateSequentialCritics(
  width: number,
  findings: readonly CriticFinding[],
): CriticPanelVerdict {
  if (!Number.isInteger(width) || width < MIN_CRITIC_WIDTH || width > MAX_CRITIC_WIDTH) {
    throw new Error(`Critic width must be an integer in ${MIN_CRITIC_WIDTH}..${MAX_CRITIC_WIDTH} (got ${width})`);
  }
  const active = findings.slice(0, width);
  const bannedPatternHits: string[] = [];
  const mergedIssues: string[] = [];

  for (const f of active) {
    for (const rule of BANNED_CRITIC_PATTERNS) {
      if (rule.re.test(f.rationale)) {
        bannedPatternHits.push(`${f.criticId}:${rule.code}`);
      }
    }
    for (const issue of f.issues) {
      if (issue.trim()) mergedIssues.push(`[${f.criticId}] ${issue.trim()}`);
    }
  }

  const allApproved =
    active.length === width &&
    active.every((f) => f.approved) &&
    mergedIssues.length === 0 &&
    bannedPatternHits.length === 0;

  return {
    width,
    approved: allApproved,
    revisionsAllowed: 1,
    bannedPatternHits,
    mergedIssues,
  };
}