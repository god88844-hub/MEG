// P3-02: Index-Separated Rule Workspace (c_0 + compact index), Gear-Aware Memory Steering,
// Stanford 4-Agent ACE Append-Only LEARNINGS.md Librarian (<=30 promoted standing rules),
// [INSURANCE] vs [OPTIMIZATION] Rule Gardening & Offline Dreaming Pass (linked wiki/).

import fs from "node:fs";
import path from "node:path";
import type { HarnessGear } from "./fusion";

export type RuleCategory = "INSURANCE" | "OPTIMIZATION";

export interface IndexedRule {
  id: string;
  category: RuleCategory;
  summary: string;
  body: string;
  verifiedUses: number;
  obsolete?: boolean;
}

/** Index-separated rule workspace (`c_0` immutable core + compact index table). */
export class RuleWorkspace {
  readonly c0: string;
  private rules = new Map<string, IndexedRule>();

  constructor(c0CorePrinciples: string) {
    this.c0 = c0CorePrinciples.trim();
  }

  registerRule(rule: IndexedRule): void {
    this.rules.set(rule.id, { ...rule });
  }

  /** Compact prompt view: `c_0` + 1-line-per-rule index (bodies stay out of base context). */
  renderCompactPrompt(): string {
    const indexLines = [...this.rules.values()]
      .sort((a, b) => a.id.localeCompare(b.id))
      .map((r) => `- [${r.category}] ${r.id}: ${r.summary}`);
    return `${this.c0}\n\n## Rule Index\n${indexLines.join("\n")}`;
  }

  fetchRuleBody(id: string): string | null {
    return this.rules.get(id)?.body ?? null;
  }

  /**
   * Rule gardening:
   * - `[INSURANCE]` rules (safety/correctness invariants) are ALWAYS preserved.
   * - `[OPTIMIZATION]` rules marked obsolete (e.g., after a model upgrade) are pruned.
   */
  gardenRules(): { kept: IndexedRule[]; prunedIds: string[] } {
    const kept: IndexedRule[] = [];
    const prunedIds: string[] = [];
    for (const rule of this.rules.values()) {
      if (rule.category === "OPTIMIZATION" && rule.obsolete) {
        prunedIds.push(rule.id);
        this.rules.delete(rule.id);
      } else {
        kept.push({ ...rule });
      }
    }
    return { kept, prunedIds };
  }
}

export interface MemorySteeringPlan {
  gear: HarnessGear;
  mode: "open_salience" | "mandatory_prehook";
  mandatoryPreHooks: string[];
}

/**
 * Gear-aware memory steering:
 * - Gear 3 (Frontier): open salience (no rigid pre-hook overhead).
 * - Gear 1-2 (Local/Mid): mandatory `Read-Memory` + `Test-Theory` pre-hooks.
 */
export function steerMemoryForGear(gear: HarnessGear): MemorySteeringPlan {
  if (gear === 3) {
    return { gear, mode: "open_salience", mandatoryPreHooks: [] };
  }
  return {
    gear,
    mode: "mandatory_prehook",
    mandatoryPreHooks: ["Read-Memory", "Test-Theory"],
  };
}

export const MAX_PROMOTED_STANDING_RULES = 30;

export interface LearningEntry {
  category: RuleCategory;
  rule: string;
  evidence: string;
}

/** Append-only `LEARNINGS.md` Librarian (Stanford 4-Agent ACE pattern). */
export function appendLearning(workspace: string, entry: LearningEntry): string {
  const file = path.join(workspace, "LEARNINGS.md");
  if (!fs.existsSync(file)) {
    fs.writeFileSync(file, "# LEARNINGS\n\n", "utf8");
  }
  const line = `- [${entry.category}] ${entry.rule.trim()} *(evidence: ${entry.evidence.trim()})*\n`;
  fs.appendFileSync(file, line, "utf8");
  return file;
}

/** Promote up to <=30 deduplicated lines from `LEARNINGS.md` into standing rules (INSURANCE first). */
export function promoteStandingRules(
  workspace: string,
  maxLines = MAX_PROMOTED_STANDING_RULES,
): string[] {
  const file = path.join(workspace, "LEARNINGS.md");
  if (!fs.existsSync(file)) return [];
  const cap = Math.min(maxLines, MAX_PROMOTED_STANDING_RULES);
  const rawLines = fs
    .readFileSync(file, "utf8")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.startsWith("- ["));

  const seen = new Set<string>();
  const insurance: string[] = [];
  const optimization: string[] = [];
  for (const line of rawLines) {
    const key = line.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    if (line.startsWith("- [INSURANCE]")) insurance.push(line);
    else optimization.push(line);
  }
  return [...insurance, ...optimization].slice(0, cap);
}

export interface DreamingNote {
  slug: string;
  title: string;
  claim: string;
  expectedEvidence: string;
  links?: string[];
}

export interface DreamingReport {
  confirmed: string[];
  contradicted: string[];
  wikiIndexFile: string;
  wikiPages: string[];
}

/**
 * Offline Dreaming pass:
 * 1. Cross-checks notes against raw transcripts (verifying `expectedEvidence`).
 * 2. Builds a linked `wiki/` directory (`wiki/index.md` + `wiki/<slug>.md` with `[[links]]`).
 */
export function runOfflineDreamingPass(
  workspace: string,
  rawTranscripts: readonly string[],
  notes: readonly DreamingNote[],
): DreamingReport {
  const corpus = rawTranscripts.join("\n").toLowerCase();
  const wikiDir = path.join(workspace, "wiki");
  fs.mkdirSync(wikiDir, { recursive: true });

  const confirmed: string[] = [];
  const contradicted: string[] = [];
  const wikiPages: string[] = [];

  for (const note of notes) {
    const safeSlug = note.slug.replace(/[^a-zA-Z0-9_-]/g, "-").toLowerCase();
    const grounded = corpus.includes(note.expectedEvidence.toLowerCase());
    if (grounded) confirmed.push(safeSlug);
    else contradicted.push(safeSlug);

    const status = grounded ? "CONFIRMED" : "UNVERIFIED_AGAINST_TRANSCRIPT";
    const linkLine =
      note.links && note.links.length > 0
        ? `Related: ${note.links.map((l) => `[[${l}]]`).join(", ")}\n`
        : "";
    const pageContent = [
      `# ${note.title}`,
      ``,
      `- Status: **${status}**`,
      `- Evidence probe: \`${note.expectedEvidence}\``,
      ``,
      note.claim,
      ``,
      linkLine,
    ].join("\n");
    const pageFile = path.join(wikiDir, `${safeSlug}.md`);
    fs.writeFileSync(pageFile, pageContent, "utf8");
    wikiPages.push(pageFile);
  }

  const wikiIndexFile = path.join(wikiDir, "index.md");
  const indexContent = [
    "# MEG Dreaming Wiki",
    "",
    ...notes.map((n) => {
      const slug = n.slug.replace(/[^a-zA-Z0-9_-]/g, "-").toLowerCase();
      const tag = confirmed.includes(slug) ? "confirmed" : "needs-check";
      return `- [[${slug}]] -- ${n.title} (${tag})`;
    }),
    "",
  ].join("\n");
  fs.writeFileSync(wikiIndexFile, indexContent, "utf8");

  return { confirmed, contradicted, wikiIndexFile, wikiPages };
}