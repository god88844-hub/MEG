// P5-05: GitHub IssueBot 5-Stage Graph, 5-Category CI Triage & PR Hygiene:
// - IssueBot 5-Stage Graph:
//   1. Tool-less Planner reads untrusted issue -> produces code-validated contract
//   2. Raw untrusted issue text is DELETED before privileged stages (Lethal Trifecta broken)
//   3. Worktree Builder executes only against validated contract
//   4. Parallel Verify (test + lint + typecheck) + Cold Reviewer
//   5. Idempotent Draft PR -> 4-tick CI watch -> Human merge (autonomous merge blocked)
// - 5-Category CI Triage (env | flake | bug | dep | infra)
// - 4-tick PR watcher (Watching 1/3..3/3 -> Ready), conflict-only rebase,
//   Qodo approval invalidation on new commits, multi-PR session summary,
//   post-merge staging QA, and 2-PR missed-case rule extraction.

import { createHash } from "node:crypto";

export interface UntrustedIssue {
  issueNumber: number;
  title: string;
  body: string; // Untrusted external input -- may contain prompt injections!
  author: string;
}

export interface ValidatedIssueContract {
  issueNumber: number;
  safeSummary: string;
  allowedFiles: string[];
  acceptanceCriteria: string[];
  idempotencyKey: string;
  rawIssueDeleted: true;
  injectionPatternsStripped: string[];
}

const INJECTION_PATTERNS: readonly { name: string; re: RegExp }[] = [
  { name: "ignore_instructions", re: /ignore (all )?(previous|prior) instructions/i },
  { name: "exfil_curl", re: /\b(curl|wget)\b[^\n]*\b(\.env|token|secret|passwd)\b/i },
  { name: "shell_injection", re: /\$\([^)]*\)|`[^`]+`|;\s*(rm\s+-rf|cat\s+\.env)/i },
  { name: "auto_merge_trick", re: /\bgh\s+pr\s+merge\b|\bgit\s+push\s+--force\b/i },
];

const SAFE_REL_PATH_RE = /^[a-zA-Z0-9._/-]+$/;

/**
 * Stage 1 -> Stage 2 of IssueBot:
 * Tool-less Planner extracts a structured proposal from an untrusted issue;
 * code validation strips unsafe paths & injection directives, then deletes the raw issue text
 * so downstream privileged Builder/Verify/PR stages NEVER see untrusted prose (Lethal Trifecta broken).
 */
export function compileIssueContractAndBreakTrifecta(
  issue: UntrustedIssue,
  toollessPlannerOutput: {
    summary: string;
    proposedFiles: string[];
    acceptanceCriteria: string[];
  },
): ValidatedIssueContract {
  const injectionPatternsStripped: string[] = [];
  const rawCombined = `${issue.title}\n${issue.body}\n${toollessPlannerOutput.summary}`;
  for (const pat of INJECTION_PATTERNS) {
    if (pat.re.test(rawCombined)) {
      injectionPatternsStripped.push(pat.name);
    }
  }

  let safeSummary = toollessPlannerOutput.summary;
  for (const pat of INJECTION_PATTERNS) {
    safeSummary = safeSummary.replace(pat.re, "[STRIPPED_UNTRUSTED_DIRECTIVE]");
  }
  safeSummary = safeSummary.trim().slice(0, 400);
  if (!safeSummary) {
    throw new Error("ISSUEBOT_CONTRACT_INVALID: empty safeSummary");
  }

  const allowedFiles = toollessPlannerOutput.proposedFiles
    .map((f) => f.replace(/\\/g, "/").trim())
    .filter(
      (f) =>
        f.length > 0 &&
        SAFE_REL_PATH_RE.test(f) &&
        !f.includes("..") &&
        !f.startsWith("/") &&
        !f.includes(".env") &&
        !f.startsWith(".github/"),
    );

  if (allowedFiles.length === 0) {
    throw new Error("ISSUEBOT_CONTRACT_INVALID: no safe workspace-relative allowedFiles");
  }
  const acceptanceCriteria = toollessPlannerOutput.acceptanceCriteria
    .map((c) => c.trim())
    .filter((c) => c.length > 0 && !INJECTION_PATTERNS.some((p) => p.re.test(c)));
  if (acceptanceCriteria.length === 0) {
    throw new Error("ISSUEBOT_CONTRACT_INVALID: acceptanceCriteria empty after sanitization");
  }

  const idempotencyKey = `issuebot-${issue.issueNumber}-${createHash("sha256")
    .update(`${issue.issueNumber}|${allowedFiles.join(",")}`)
    .digest("hex")
    .slice(0, 10)}`;

  // Raw issue.body is intentionally NOT stored on the returned contract.
  return {
    issueNumber: issue.issueNumber,
    safeSummary,
    allowedFiles,
    acceptanceCriteria,
    idempotencyKey,
    rawIssueDeleted: true,
    injectionPatternsStripped,
  };
}

export interface IssueBotRunResult {
  stagesExecuted: string[];
  contract: ValidatedIssueContract;
  builderReceivedRawIssue: false;
  parallelVerify: { test: boolean; lint: boolean; typecheck: boolean };
  coldReviewerApproved: boolean;
  draftPr: { key: string; created: boolean; draft: true; merged: false; awaitingHumanMerge: true };
}

/**
 * Execute the full 5-Stage IssueBot Graph with idempotent Draft PR creation and human-only merge.
 */
export async function runIssueBotGraph(
  issue: UntrustedIssue,
  hooks: {
    toollessPlan: (untrusted: UntrustedIssue) => Promise<{
      summary: string;
      proposedFiles: string[];
      acceptanceCriteria: string[];
    }>;
    buildInWorktree: (contract: ValidatedIssueContract) => Promise<{ editedFiles: string[] }>;
    verifyTest: () => Promise<boolean>;
    verifyLint: () => Promise<boolean>;
    verifyTypecheck: () => Promise<boolean>;
    coldReview: (contract: ValidatedIssueContract, editedFiles: string[]) => Promise<boolean>;
    existingDraftPrKeys?: Set<string>;
  },
): Promise<IssueBotRunResult> {
  const stagesExecuted: string[] = [];

  // Stage 1: Tool-less Planner reads untrusted issue
  const rawPlan = await hooks.toollessPlan(issue);
  stagesExecuted.push("1_toolless_planner");

  // Stage 2: Code-validate contract & delete raw issue (breaks Lethal Trifecta)
  const contract = compileIssueContractAndBreakTrifecta(issue, rawPlan);
  stagesExecuted.push("2_contract_validated_raw_deleted");

  // Stage 3: Worktree Builder sees ONLY validated contract
  const built = await hooks.buildInWorktree(contract);
  stagesExecuted.push("3_worktree_builder");

  // Stage 4: Parallel Verify (test + lint + typecheck) + Cold Reviewer
  const [testOk, lintOk, typecheckOk] = await Promise.all([
    hooks.verifyTest(),
    hooks.verifyLint(),
    hooks.verifyTypecheck(),
  ]);
  if (!testOk || !lintOk || !typecheckOk) {
    throw new Error(`ISSUEBOT_VERIFY_FAILED: test=${testOk} lint=${lintOk} typecheck=${typecheckOk}`);
  }
  const coldReviewerApproved = await hooks.coldReview(contract, built.editedFiles);
  if (!coldReviewerApproved) {
    throw new Error("ISSUEBOT_REVIEW_REJECTED: cold reviewer withheld approval");
  }
  stagesExecuted.push("4_parallel_verify_and_cold_review");

  // Stage 5: Idempotent Draft PR (never auto-merges)
  const existing = hooks.existingDraftPrKeys ?? new Set<string>();
  const created = !existing.has(contract.idempotencyKey);
  existing.add(contract.idempotencyKey);
  stagesExecuted.push("5_idempotent_draft_pr_human_merge_required");

  return {
    stagesExecuted,
    contract,
    builderReceivedRawIssue: false,
    parallelVerify: { test: testOk, lint: lintOk, typecheck: typecheckOk },
    coldReviewerApproved,
    draftPr: {
      key: contract.idempotencyKey,
      created,
      draft: true,
      merged: false,
      awaitingHumanMerge: true,
    },
  };
}

export type CiFailureCategory = "env" | "flake" | "bug" | "dep" | "infra";

/** 5-Category CI Triage (`env | flake | bug | dep | infra`). */
export function triageCiFailure(logText: string): { category: CiFailureCategory; action: string } {
  const low = logText.toLowerCase();
  if (/(missing env|environment variable|permission denied|enoent:.*config|toolchain not installed)/.test(low)) {
    return { category: "env", action: "Fix runner environment setup or missing env config" };
  }
  if (/(flaky|timed out waiting|etimedout|intermittent|race condition|passed on retry)/.test(low)) {
    return { category: "flake", action: "Quarantine/stabilize non-deterministic test before re-running" };
  }
  if (/(npm err!.*404|could not resolve dependency|lockfile|peer dep|cargo.*version.*not found|modulenotfounderror)/.test(low)) {
    return { category: "dep", action: "Pin/update dependency and refresh lockfile" };
  }
  if (/(runner lost communication|503 service unavailable|out of disk space|rate limit exceeded|github actions.*outage)/.test(low)) {
    return { category: "infra", action: "Retry workflow after CI runner infrastructure recovers" };
  }
  return { category: "bug", action: "Reproduce locally and fix code defect" };
}

/** 4-Tick PR Watcher (`Watching 1/3`, `Watching 2/3`, `Watching 3/3`, `Ready`). */
export class PrCiWatcher {
  private tickCount = 0;

  tick(ciGreen: boolean): { status: string; ready: boolean } {
    if (!ciGreen) {
      this.tickCount = 0;
      return { status: "Failed (reset watcher)", ready: false };
    }
    this.tickCount += 1;
    if (this.tickCount <= 3) {
      return { status: `Watching ${this.tickCount}/3`, ready: false };
    }
    return { status: "Ready", ready: true };
  }
}

/** Conflict-only rebase decision: rebase ONLY when merge conflicts exist (avoids unnecessary SHA churn). */
export function shouldRebasePr(hasMergeConflict: boolean, behindCommits: number): { rebase: boolean; reason: string } {
  if (hasMergeConflict) {
    return { rebase: true, reason: "Conflict-only rebase: PR has merge conflicts with base branch" };
  }
  return {
    rebase: false,
    reason: `No conflicts (${behindCommits} commit(s) behind); skipping rebase to preserve review/CI state`,
  };
}

export interface QodoApprovalState {
  approvedCommitSha: string;
  currentHeadSha: string;
  valid: boolean;
  reason: string;
}

/** Qodo approval invalidation: any new commit after approval invalidates prior approval. */
export function checkQodoApprovalValidity(
  approvedCommitSha: string | null,
  currentHeadSha: string,
): QodoApprovalState {
  if (!approvedCommitSha) {
    return { approvedCommitSha: "", currentHeadSha, valid: false, reason: "No prior approval recorded" };
  }
  if (approvedCommitSha !== currentHeadSha) {
    return {
      approvedCommitSha,
      currentHeadSha,
      valid: false,
      reason: `Qodo approval invalidated: HEAD moved (${approvedCommitSha.slice(0, 7)} -> ${currentHeadSha.slice(0, 7)})`,
    };
  }
  return { approvedCommitSha, currentHeadSha, valid: true, reason: "Approval matches current HEAD" };
}

export interface PrSessionRecord {
  prNumber: number;
  title: string;
  status: "draft" | "ready" | "merged" | "blocked";
  ciStatus: "green" | "red" | "pending";
  stagingQaPassed?: boolean;
  missedCaseTag?: string;
}

/** Multi-PR session summary + post-merge staging QA audit + 2-PR missed-case rule extraction. */
export function buildPrSessionHygieneReport(prs: readonly PrSessionRecord[]): {
  summaryMarkdown: string;
  pendingStagingQaPrs: number[];
  extractedRules: string[];
} {
  const rows = prs.map(
    (p) =>
      `| #${p.prNumber} | ${p.title} | ${p.status} | ${p.ciStatus} | ${p.stagingQaPassed === undefined ? "n/a" : p.stagingQaPassed ? "pass" : "pending"} |`,
  );
  const summaryMarkdown = [
    "| PR | Title | Status | CI | Staging QA |",
    "| :--- | :--- | :--- | :--- | :--- |",
    ...rows,
  ].join("\n");

  const pendingStagingQaPrs = prs
    .filter((p) => p.status === "merged" && p.stagingQaPassed !== true)
    .map((p) => p.prNumber);

  // 2-PR missed-case rule extraction: any missedCaseTag recurring across >=2 PRs becomes a standing rule.
  const tagCounts = new Map<string, number>();
  for (const pr of prs) {
    if (pr.missedCaseTag) {
      tagCounts.set(pr.missedCaseTag, (tagCounts.get(pr.missedCaseTag) ?? 0) + 1);
    }
  }
  const extractedRules: string[] = [];
  for (const [tag, count] of tagCounts.entries()) {
    if (count >= 2) {
      extractedRules.push(`[RULE FROM ${count} PRs] Add automated check for recurring missed case: '${tag}'`);
    }
  }

  return { summaryMarkdown, pendingStagingQaPrs, extractedRules };
}