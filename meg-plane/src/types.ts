// Canonical schemas, contracts, and runtime validators for the MEG control plane (P2-01).

import { createHash } from "node:crypto";

export const TOOL_ERROR_CATEGORIES = [
  "Transient",
  "InvalidArguments",
  "UnexpectedEnvironment",
  "ProviderError",
  "Timeout",
  "UserAborted",
] as const;

export type ToolErrorCategory = (typeof TOOL_ERROR_CATEGORIES)[number];

export interface PreFlightTaskContract {
  taskId: string;
  goal: string;
  allowedPaths: string[];
  protectedPaths: string[];
  verificationCmd: string;
  expectedExitCode: number;
  maxTurns: number;
  createdAt: number;
}

export interface MonotonicFeatureItem {
  id: string;
  description: string;
  passed: boolean;
  readOnly: boolean;
  verifiedAt?: number;
  evidenceHash?: string;
}

export interface FeaturesManifest {
  version: 1;
  items: MonotonicFeatureItem[];
}

export interface ExecutionReceipt {
  version: 1;
  taskId: string;
  treeHash: string;
  checksPassed: boolean;
  receiptHash: string;
  verifiedAt: number;
}

// 4-Card Cookbook schemas: Planner, Builder, Reviewer, Kickoff
export interface PlannerCard {
  card: "Planner";
  taskId: string;
  steps: string[];
  targetFiles: string[];
  risks: string[];
}

export interface BuilderCard {
  card: "Builder";
  taskId: string;
  branch: string;
  editedFiles: string[];
  verificationCmd: string;
  summary: string;
}

export interface ReviewerCard {
  card: "Reviewer";
  taskId: string;
  reviewerModel: string;
  approved: boolean;
  findings: string[];
}

export interface KickoffCard {
  card: "Kickoff";
  taskId: string;
  goal: string;
  constraints: string[];
  assumptionsFile?: string;
}

export type CookbookCard = PlannerCard | BuilderCard | ReviewerCard | KickoffCard;

// 7-field Swarm Handoff contract
export interface SwarmHandoff {
  handoffId: string;
  role: "planner" | "researcher" | "reviewer" | "builder";
  objective: string;
  allowedFiles: string[];
  constraints: string[];
  acceptanceCriteria: string[];
  budget: { maxTurns: number; maxTokens: number };
}

// HarnessMM Executable Decision Graph node
export interface EDGNode {
  id: string;
  triggerPattern: string;
  toolSequence: string[];
  preconditions: string[];
  cachedAt: number;
  hitCount: number;
}

// Substitution Paradox gate certificate
export interface SubstitutionCertificate {
  nodeId: string;
  eliminatedReasoningTokens: number;
  authorshipCostTokens: number;
  selectionCostTokens: number;
  recoveryCostTokens: number;
  approved: boolean;
}

// GOAL.md structured schema
export interface GoalSpec {
  id: string;
  title: string;
  objective: string;
  metrics: string[];
  floorInvariants: string[];
  budgetUsd: number;
  status: "active" | "paused" | "completed" | "cleared";
}

export function sha256Hex(input: string | Uint8Array): string {
  return createHash("sha256").update(input).digest("hex");
}

export function signExecutionReceipt(
  taskId: string,
  treeHash: string,
  checksPassed: boolean,
  verifiedAt = Date.now(),
): ExecutionReceipt {
  const receiptHash = sha256Hex(`v1|${taskId}|${treeHash}|${checksPassed}`);
  return {
    version: 1,
    taskId,
    treeHash,
    checksPassed,
    receiptHash,
    verifiedAt,
  };
}

export function verifyExecutionReceipt(
  receipt: ExecutionReceipt | null | undefined,
  expectedTaskId?: string,
  expectedTreeHash?: string,
): { valid: boolean; reason?: string } {
  if (!receipt || receipt.version !== 1) {
    return { valid: false, reason: "Missing or invalid receipt version" };
  }
  if (!receipt.taskId || !receipt.treeHash || !receipt.receiptHash) {
    return { valid: false, reason: "Receipt missing required fields" };
  }
  if (!receipt.checksPassed) {
    return { valid: false, reason: "Receipt records checksPassed = false" };
  }
  if (expectedTaskId && receipt.taskId !== expectedTaskId) {
    return { valid: false, reason: `Receipt taskId mismatch (${receipt.taskId} != ${expectedTaskId})` };
  }
  if (expectedTreeHash && receipt.treeHash !== expectedTreeHash) {
    return { valid: false, reason: "Receipt treeHash does not match current workspace tree" };
  }
  const expected = sha256Hex(`v1|${receipt.taskId}|${receipt.treeHash}|${receipt.checksPassed}`);
  if (receipt.receiptHash !== expected) {
    return { valid: false, reason: "Receipt hash signature mismatch" };
  }
  return { valid: true };
}

/** Enforce monotonic feature progression: once passed & readOnly, an item cannot silently regress. */
export function updateMonotonicFeature(
  manifest: FeaturesManifest,
  id: string,
  nextPassed: boolean,
  evidenceHash?: string,
): FeaturesManifest {
  const idx = manifest.items.findIndex((i) => i.id === id);
  if (idx === -1) throw new Error(`Unknown feature id: ${id}`);
  const cur = manifest.items[idx]!;
  if (cur.passed && cur.readOnly && !nextPassed) {
    throw new Error(`Monotonic feature '${id}' is locked readOnly after passing; cannot regress`);
  }
  const updated: MonotonicFeatureItem = {
    ...cur,
    passed: cur.passed || nextPassed,
    readOnly: cur.readOnly || nextPassed,
    verifiedAt: nextPassed ? Date.now() : cur.verifiedAt,
    evidenceHash: evidenceHash ?? cur.evidenceHash,
  };
  const items = [...manifest.items];
  items[idx] = updated;
  return { version: 1, items };
}

/** Substitution Paradox check: eliminated_reasoning > authorship + selection + recovery. */
export function evaluateSubstitutionCertificate(
  nodeId: string,
  eliminatedReasoningTokens: number,
  authorshipCostTokens: number,
  selectionCostTokens: number,
  recoveryCostTokens: number,
): SubstitutionCertificate {
  const overhead = authorshipCostTokens + selectionCostTokens + recoveryCostTokens;
  return {
    nodeId,
    eliminatedReasoningTokens,
    authorshipCostTokens,
    selectionCostTokens,
    recoveryCostTokens,
    approved: eliminatedReasoningTokens > overhead,
  };
}

export function isValidToolErrorCategory(value: string): value is ToolErrorCategory {
  return (TOOL_ERROR_CATEGORIES as readonly string[]).includes(value);
}

export function validateSwarmHandoff(h: Partial<SwarmHandoff>): SwarmHandoff {
  if (!h.handoffId || !h.objective || !h.role) {
    throw new Error("SwarmHandoff requires handoffId, role, and objective");
  }
  if (!Array.isArray(h.allowedFiles) || !Array.isArray(h.constraints) || !Array.isArray(h.acceptanceCriteria)) {
    throw new Error("SwarmHandoff requires allowedFiles, constraints, and acceptanceCriteria arrays");
  }
  if (!h.budget || h.budget.maxTurns <= 0 || h.budget.maxTokens <= 0) {
    throw new Error("SwarmHandoff requires positive budget.maxTurns and budget.maxTokens");
  }
  return h as SwarmHandoff;
}