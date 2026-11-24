// P3-01: 4-Part Memory Stack & Bi-Temporal Ledger:
// - Authoritative event ledger + lexical workspace search (no code vector DB)
// - STAIR ToC leaf retriever for long Markdown docs
// - Reglos bi-temporal records (valid_time + learning_time, interval closing)
// - 5 epistemic states (KNOWS | BELIEVES | TRIED | VERIFIED | UNESTABLISHED)
// - SHA-256 source-file hash staleness invalidation
// - HYRA 2.0 3-tier Books (Observation -> Verified -> Convention [human approved])
// - Grok Bot Fast-Fact rule blocking caching of volatile live state

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export type EpistemicState =
  | "KNOWS"
  | "BELIEVES"
  | "TRIED"
  | "VERIFIED"
  | "UNESTABLISHED";

export type BookTier = "Observation" | "Verified" | "Convention";

export const VOLATILE_FAST_FACT_FIELDS = [
  "price",
  "plan_tier",
  "ci_status",
  "assignee",
  "balance",
] as const;

const VOLATILE_KEY_RE =
  /(^|[.:/_-])(price|plan[_-]?tier|ci[_-]?status|assignee|balance)($|[.:/_-])/i;

/** Grok Bot Fast-Fact rule: volatile live state must be fetched live, never cached. */
export function isVolatileFastFactKey(key: string): boolean {
  return VOLATILE_KEY_RE.test(key.trim());
}

export interface BiTemporalRecord {
  id: string;
  key: string;
  value: string;
  epistemic: EpistemicState;
  bookTier: BookTier;
  validFrom: number;
  validTo: number | null;
  learnedAt: number;
  sourceFile?: string;
  sourceSha256?: string;
}

function sha256File(filePath: string): string {
  return createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

export class BiTemporalMemoryStore {
  private records: BiTemporalRecord[] = [];
  private seq = 0;
  private readonly ledgerFile: string | null;

  constructor(ledgerFile: string | null = null) {
    this.ledgerFile = ledgerFile;
  }

  private appendLedger(event: Record<string, unknown>): void {
    if (!this.ledgerFile) return;
    fs.mkdirSync(path.dirname(this.ledgerFile), { recursive: true });
    fs.appendFileSync(this.ledgerFile, JSON.stringify(event) + "\n", "utf8");
  }

  /**
   * Record a fact bi-temporally: closes any active interval on `key` rather than
   * overwriting history, hashes `sourceFile` when provided, and blocks volatile live fields.
   */
  recordFact(input: {
    key: string;
    value: string;
    epistemic: EpistemicState;
    bookTier?: BookTier;
    validFrom?: number;
    learnedAt?: number;
    workspace?: string;
    sourceFile?: string;
  }): BiTemporalRecord {
    if (isVolatileFastFactKey(input.key)) {
      throw new Error(
        `FAST_FACT_BLOCKED: refusing to cache volatile live state key '${input.key}'`,
      );
    }
    const tier = input.bookTier ?? "Observation";
    if (tier === "Convention") {
      throw new Error(
        "HYRA_POLICY: cannot insert directly as Convention; promote Observation -> Verified -> Convention with humanApproved: true",
      );
    }

    const now = input.learnedAt ?? Date.now();
    const validFrom = input.validFrom ?? now;

    // Close existing open interval for this key (Reglos bi-temporal preservation).
    for (const rec of this.records) {
      if (rec.key === input.key && rec.validTo === null) {
        rec.validTo = validFrom;
        this.appendLedger({ type: "close_interval", id: rec.id, key: rec.key, validTo: validFrom });
      }
    }

    let sourceSha256: string | undefined;
    if (input.sourceFile && input.workspace) {
      const full = path.resolve(input.workspace, input.sourceFile);
      if (fs.existsSync(full) && fs.statSync(full).isFile()) {
        sourceSha256 = sha256File(full);
      }
    }

    const record: BiTemporalRecord = {
      id: `mem-${++this.seq}`,
      key: input.key,
      value: input.value,
      epistemic: input.epistemic,
      bookTier: tier,
      validFrom,
      validTo: null,
      learnedAt: now,
      sourceFile: input.sourceFile,
      sourceSha256,
    };
    this.records.push(record);
    this.appendLedger({ type: "record_fact", record });
    return { ...record };
  }

  /** Query history for a key (all closed + open intervals). */
  history(key: string): BiTemporalRecord[] {
    return this.records.filter((r) => r.key === key).map((r) => ({ ...r }));
  }

  /** Bi-temporal point-in-time query: what was valid at `validAt` as known at `learnedAt`. */
  queryAsOf(key: string, validAt: number, learnedAt = Number.MAX_SAFE_INTEGER): BiTemporalRecord | null {
    const matches = this.records.filter(
      (r) =>
        r.key === key &&
        r.learnedAt <= learnedAt &&
        r.validFrom <= validAt &&
        (r.validTo === null || validAt < r.validTo),
    );
    const latest = matches.at(-1);
    return latest ? { ...latest } : null;
  }

  /** SHA-256 source-file hash staleness invalidation. */
  isStale(id: string, workspace: string): boolean {
    const rec = this.records.find((r) => r.id === id);
    if (!rec) return true;
    if (!rec.sourceFile || !rec.sourceSha256) return false;
    const full = path.resolve(workspace, rec.sourceFile);
    if (!fs.existsSync(full) || !fs.statSync(full).isFile()) return true;
    return sha256File(full) !== rec.sourceSha256;
  }

  /** HYRA 2.0 3-tier Books promotion: Observation -> Verified -> Convention (human approved). */
  promoteBook(
    id: string,
    target: BookTier,
    options: { humanApproved?: boolean } = {},
  ): BiTemporalRecord {
    const rec = this.records.find((r) => r.id === id);
    if (!rec) throw new Error(`Unknown memory record: ${id}`);
    if (target === "Verified") {
      if (rec.bookTier !== "Observation" && rec.bookTier !== "Verified") {
        throw new Error(`Invalid promotion ${rec.bookTier} -> Verified`);
      }
      rec.bookTier = "Verified";
      rec.epistemic = "VERIFIED";
    } else if (target === "Convention") {
      if (rec.bookTier !== "Verified") {
        throw new Error(`HYRA_POLICY: must be Verified before promoting to Convention (currently ${rec.bookTier})`);
      }
      if (!options.humanApproved) {
        throw new Error("HYRA_POLICY: promoting to Convention requires humanApproved: true");
      }
      rec.bookTier = "Convention";
    }
    this.appendLedger({ type: "promote_book", id: rec.id, bookTier: rec.bookTier });
    return { ...rec };
  }
}

export interface LexicalMatch {
  file: string;
  line: number;
  text: string;
}

const IGNORED_DIRS = new Set([".git", "node_modules", "target", ".meg", "__pycache__", ".venv"]);

/** Lexical search first across workspace source files (zero vector DB dependency). */
export function lexicalSearchWorkspace(
  workspace: string,
  pattern: string | RegExp,
  maxHits = 50,
): LexicalMatch[] {
  const re = typeof pattern === "string" ? new RegExp(pattern, "i") : pattern;
  const hits: LexicalMatch[] = [];

  const walk = (dir: string) => {
    if (hits.length >= maxHits) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (hits.length >= maxHits) return;
      if (entry.isDirectory()) {
        if (!IGNORED_DIRS.has(entry.name)) walk(path.join(dir, entry.name));
        continue;
      }
      const full = path.join(dir, entry.name);
      if (fs.statSync(full).size > 512 * 1024) continue;
      let raw: string;
      try {
        raw = fs.readFileSync(full, "utf8");
      } catch {
        continue;
      }
      if (raw.includes("\u0000")) continue;
      const lines = raw.split(/\r?\n/);
      const rel = path.relative(workspace, full).replace(/\\/g, "/");
      for (let i = 0; i < lines.length; i++) {
        if (re.test(lines[i]!)) {
          hits.push({ file: rel, line: i + 1, text: lines[i]!.trim().slice(0, 160) });
          if (hits.length >= maxHits) return;
        }
      }
    }
  };
  walk(workspace);
  return hits;
}

export interface StairLeafSection {
  breadcrumb: string;
  heading: string;
  level: number;
  content: string;
  score: number;
}

/**
 * STAIR ToC leaf retriever for long Markdown documents:
 * parses heading hierarchy into leaf sections and retrieves only the top-matching leaves.
 */
export function stairTocLeafRetrieve(
  markdown: string,
  query: string,
  maxLeaves = 3,
): StairLeafSection[] {
  const lines = markdown.split(/\r?\n/);
  const stack: { level: number; title: string }[] = [];
  const rawSections: { breadcrumb: string; heading: string; level: number; body: string[]; hasChildren: boolean }[] = [];

  for (const line of lines) {
    const m = line.match(/^(#{1,6})\s+(.+?)\s*$/);
    if (m) {
      const level = m[1]!.length;
      const title = m[2]!;
      if (rawSections.length > 0 && level > rawSections[rawSections.length - 1]!.level) {
        rawSections[rawSections.length - 1]!.hasChildren = true;
      }
      while (stack.length > 0 && stack[stack.length - 1]!.level >= level) {
        stack.pop();
      }
      stack.push({ level, title });
      rawSections.push({
        breadcrumb: stack.map((s) => s.title).join(" > "),
        heading: title,
        level,
        body: [],
        hasChildren: false,
      });
    } else if (rawSections.length > 0) {
      rawSections[rawSections.length - 1]!.body.push(line);
    }
  }

  const terms = query
    .toLowerCase()
    .split(/[^a-z0-9_]+/)
    .filter((t) => t.length >= 2);

  const leaves = rawSections.filter((s) => !s.hasChildren || s.body.join("").trim().length > 0);
  const scored: StairLeafSection[] = leaves
    .map((s) => {
      const content = s.body.join("\n").trim();
      const hay = `${s.breadcrumb}\n${content}`.toLowerCase();
      let score = 0;
      for (const term of terms) {
        if (s.heading.toLowerCase().includes(term)) score += 3;
        if (hay.includes(term)) score += 1;
      }
      return { breadcrumb: s.breadcrumb, heading: s.heading, level: s.level, content, score };
    })
    .filter((s) => s.score > 0)
    .sort((a, b) => b.score - a.score);

  return scored.slice(0, maxLeaves);
}