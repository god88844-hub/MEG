// P2-06: 5-Move CONTEXT.md Compiler (Cut, Order, Fetch, Compress, Split)
// + Single-Pass Media Compaction (@media: disk pointers).

import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const CONTEXT_LIMITS = {
  MAX_TOP_RULES: 15,
  MAX_TOOLS: 6,
  MAX_EXAMPLES: 2,
  MAX_AGENTS_LINES: 100,
  MAX_MEMORY_CHARS: 2200,
  MAX_USER_CHARS: 1375,
  MAX_FETCH_HITS: 3,
  SPLIT_CHAR_THRESHOLD: 40_000,
  SPLIT_PAGE_THRESHOLD: 20,
  ACTIVE_CONTEXT_RATIO_CAP: 0.40,
  TAIL_ANCHOR_INTERVAL_TURNS: 4,
} as const;

export interface StaticContextInput {
  rules: string[];
  tools: { name: string; schema: string }[];
  examples: string[];
  agentsMd?: string;
  memoryMd?: string;
  userMd?: string;
  todoMd?: string;
  pins?: string[];
  goal: string;
  turnNumber: number;
}

export interface CompiledContext {
  staticPrefix: string;
  prefixHash: string;
  dynamicTail: string;
  fullPrompt: string;
  cutReport: {
    rulesKept: number;
    toolsKept: number;
    examplesKept: number;
    agentsLinesKept: number;
    memoryCharsKept: number;
    userCharsKept: number;
  };
  tailGoalAnchored: boolean;
}

/** Strip HTML comments `<!-- ... -->` so comments never invalidate KV cache prefixes. */
export function stripHtmlComments(text: string): string {
  return text.replace(/<!--[\s\S]*?-->/g, "").replace(/\n{3,}/g, "\n\n").trim();
}

function capLines(text: string, maxLines: number): string {
  const lines = text.split(/\r?\n/);
  if (lines.length <= maxLines) return text;
  return lines.slice(0, maxLines).join("\n");
}

function capChars(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text;
  return text.slice(0, maxChars);
}

/**
 * Move 1 (Cut) + Move 2 (Order):
 * - Enforces strict rule/tool/example/AGENTS/MEMORY/USER caps.
 * - Byte-sorts static blocks and strips `<!-- -->` comments for >90% KV cache stability.
 * - Places `todo.md` + `PIN` recency override + `GOAL` at the very bottom, plus
 *   injects a Tail Goal Anchor every 4 turns.
 */
export function compileContext(input: StaticContextInput): CompiledContext {
  const rules = input.rules.slice(0, CONTEXT_LIMITS.MAX_TOP_RULES).map(stripHtmlComments);
  const tools = [...input.tools]
    .slice(0, CONTEXT_LIMITS.MAX_TOOLS)
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  const examples = input.examples.slice(0, CONTEXT_LIMITS.MAX_EXAMPLES).map(stripHtmlComments);

  const agents = input.agentsMd
    ? capLines(stripHtmlComments(input.agentsMd), CONTEXT_LIMITS.MAX_AGENTS_LINES)
    : "";
  const memory = input.memoryMd
    ? capChars(stripHtmlComments(input.memoryMd), CONTEXT_LIMITS.MAX_MEMORY_CHARS)
    : "";
  const user = input.userMd
    ? capChars(stripHtmlComments(input.userMd), CONTEXT_LIMITS.MAX_USER_CHARS)
    : "";

  // Byte-sorted static sections keep KV-cache prefix invariant across turns.
  const staticBlocks: [string, string][] = [
    ["01_AGENTS", agents],
    ["02_RULES", rules.map((r, i) => `${i + 1}. ${r}`).join("\n")],
    ["03_TOOLS", tools.map((t) => `- ${t.name}: ${stripHtmlComments(t.schema)}`).join("\n")],
    ["04_EXAMPLES", examples.join("\n---\n")],
    ["05_MEMORY", memory],
    ["06_USER", user],
  ].filter(([, body]) => body.length > 0) as [string, string][];

  staticBlocks.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  const staticPrefix = staticBlocks.map(([k, v]) => `## ${k}\n${v}`).join("\n\n");
  const prefixHash = createHash("sha256").update(staticPrefix).digest("hex");

  const tailParts: string[] = [];
  if (input.todoMd?.trim()) {
    tailParts.push(`## TODO\n${input.todoMd.trim()}`);
  }
  if (input.pins && input.pins.length > 0) {
    tailParts.push(`## PINNED RECENCY OVERRIDES\n${input.pins.map((p) => `- [PIN] ${p}`).join("\n")}`);
  }
  const tailGoalAnchored =
    input.turnNumber > 0 && input.turnNumber % CONTEXT_LIMITS.TAIL_ANCHOR_INTERVAL_TURNS === 0;
  if (tailGoalAnchored) {
    tailParts.push(`[TAIL GOAL ANCHOR — Turn ${input.turnNumber}] Stay strictly on the original GOAL below.`);
  }
  tailParts.push(`GOAL: ${input.goal.trim()}`);

  const dynamicTail = tailParts.join("\n\n");
  return {
    staticPrefix,
    prefixHash,
    dynamicTail,
    fullPrompt: `${staticPrefix}\n\n${dynamicTail}`,
    cutReport: {
      rulesKept: rules.length,
      toolsKept: tools.length,
      examplesKept: examples.length,
      agentsLinesKept: agents ? agents.split("\n").length : 0,
      memoryCharsKept: memory.length,
      userCharsKept: user.length,
    },
    tailGoalAnchored,
  };
}

export interface FetchHit {
  path: string;
  mtimeMs: number;
  snippet: string;
}

/** Move 3 (Fetch): sort newest-first by mtime and cap at <= 3 hits. */
export function fetchNewestHits(hits: readonly FetchHit[], maxHits = CONTEXT_LIMITS.MAX_FETCH_HITS): FetchHit[] {
  return [...hits]
    .sort((a, b) => b.mtimeMs - a.mtimeMs)
    .slice(0, Math.min(maxHits, CONTEXT_LIMITS.MAX_FETCH_HITS));
}

export interface HandoffBriefInput {
  goal: string;
  completed: string[];
  filesTouched: string[];
  currentAttemptErrors: string[];
  priorAttemptTracebacks: string[];
  nextStep: string;
  freshAttempt: boolean;
  contextWindowChars: number;
}

/**
 * Move 4 (Compress — EoCC/SAC 5-part handoff brief):
 * - Keeps current-attempt errors visible.
 * - Strips raw tracebacks across fresh attempts.
 * - Enforces <40% active context cap.
 */
export function buildCompressedHandoff(input: HandoffBriefInput): string {
  const errors = input.freshAttempt
    ? input.currentAttemptErrors
    : [...input.currentAttemptErrors, ...input.priorAttemptTracebacks];
  const brief = [
    `### 1. Goal\n${input.goal}`,
    `### 2. Completed\n${input.completed.map((c) => `- ${c}`).join("\n") || "- (none)"}`,
    `### 3. Files Touched\n${input.filesTouched.map((f) => `- ${f}`).join("\n") || "- (none)"}`,
    `### 4. Active Errors\n${errors.map((e) => `- ${e}`).join("\n") || "- (none)"}`,
    `### 5. Next Step\n${input.nextStep}`,
  ].join("\n\n");

  const maxChars = Math.max(256, Math.floor(input.contextWindowChars * CONTEXT_LIMITS.ACTIVE_CONTEXT_RATIO_CAP));
  return brief.length > maxChars ? brief.slice(0, maxChars) : brief;
}

/** Move 5 (Split): route >20-page or >40K-char reads to a 5-line reader subagent brief. */
export function shouldSplitLargeRead(charCount: number, pageCount = 0): boolean {
  return charCount > CONTEXT_LIMITS.SPLIT_CHAR_THRESHOLD || pageCount > CONTEXT_LIMITS.SPLIT_PAGE_THRESHOLD;
}

export function summarizeLargeReadToFiveLines(sourcePath: string, content: string): string {
  const lines = content
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  const picked = [
    `1. Source: ${sourcePath} (${content.length} chars, ${lines.length} non-empty lines)`,
    `2. Opening: ${(lines[0] ?? "(empty)").slice(0, 120)}`,
    `3. Key section: ${(lines[Math.floor(lines.length / 3)] ?? "(n/a)").slice(0, 120)}`,
    `4. Midpoint: ${(lines[Math.floor((lines.length * 2) / 3)] ?? "(n/a)").slice(0, 120)}`,
    `5. Closing: ${(lines[lines.length - 1] ?? "(n/a)").slice(0, 120)}`,
  ];
  return picked.join("\n");
}

export interface MediaBlock {
  kind: "text" | "image" | "video_frame";
  text?: string;
  mimeType?: string;
  base64Data?: string;
  pointer?: string;
}

/**
 * Single-pass media compaction: writes processed binary images/video frames to
 * `<workspace>/.meg/media/<hash>.<ext>` and replaces inline payloads with `@media:` pointers.
 */
export function compactProcessedMedia(workspace: string, blocks: readonly MediaBlock[]): MediaBlock[] {
  const mediaDir = path.join(workspace, ".meg", "media");
  return blocks.map((block) => {
    if (block.kind === "text" || !block.base64Data) return { ...block };
    const buf = Buffer.from(block.base64Data, "base64");
    const hash = createHash("sha256").update(buf).digest("hex").slice(0, 16);
    const ext =
      block.mimeType?.includes("png")
        ? "png"
        : block.mimeType?.includes("jpeg") || block.mimeType?.includes("jpg")
          ? "jpg"
          : "bin";
    fs.mkdirSync(mediaDir, { recursive: true });
    const file = path.join(mediaDir, `${hash}.${ext}`);
    if (!fs.existsSync(file)) {
      fs.writeFileSync(file, buf);
    }
    return {
      kind: block.kind,
      mimeType: block.mimeType,
      pointer: `@media:.meg/media/${hash}.${ext}`,
    };
  });
}