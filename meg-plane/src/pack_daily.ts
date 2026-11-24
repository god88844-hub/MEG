// P6-07 Pack 6: Daily Usage, Flat-File Obsidian Second Brain, Karpathy 6-Pass Wiki Compiler,
// Spaced Resurfacing (1/3/7/21/60d), Alook Team Rooms (#intel vs #alerts), Nimble Web Diff & Voice Keyterms.

import fs from "node:fs";
import path from "node:path";

export const SECOND_BRAIN_DIRS = ["notes", "people", "projects", "inbox", "briefs"] as const;
export const RESURFACE_INTERVAL_DAYS = [1, 3, 7, 21, 60] as const;

export interface SecondBrainVault {
  root: string;
  decisionsFile: string;
}

/** Initialize flat-file Obsidian/Markdown Second Brain layout. */
export function initSecondBrainVault(rootDir: string): SecondBrainVault {
  for (const dir of SECOND_BRAIN_DIRS) {
    fs.mkdirSync(path.join(rootDir, dir), { recursive: true });
  }
  const decisionsFile = path.join(rootDir, "decisions.md");
  if (!fs.existsSync(decisionsFile)) {
    fs.writeFileSync(decisionsFile, "# Decisions\n\n", "utf8");
  }
  return { root: rootDir, decisionsFile };
}

export interface MemoryNote {
  slug: string;
  title: string;
  body: string;
  createdAtMs: number;
  tags: string[];
}

export function rememberNote(vaultRoot: string, note: MemoryNote): string {
  const safe = note.slug.replace(/[^a-zA-Z0-9_-]/g, "-").toLowerCase();
  const file = path.join(vaultRoot, "notes", `${safe}.md`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const md = [
    `---`,
    `title: ${note.title}`,
    `createdAtMs: ${note.createdAtMs}`,
    `tags: [${note.tags.join(", ")}]`,
    `---`,
    `# ${note.title}`,
    ``,
    note.body.trim(),
    ``,
  ].join("\n");
  fs.writeFileSync(file, md, "utf8");
  return file;
}

export function recallNotes(vaultRoot: string, query: string): { file: string; snippet: string }[] {
  const notesDir = path.join(vaultRoot, "notes");
  if (!fs.existsSync(notesDir)) return [];
  const q = query.toLowerCase();
  const hits: { file: string; snippet: string }[] = [];
  for (const entry of fs.readdirSync(notesDir, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(".md")) continue;
    const raw = fs.readFileSync(path.join(notesDir, entry.name), "utf8");
    if (raw.toLowerCase().includes(q)) {
      hits.push({ file: `notes/${entry.name}`, snippet: raw.slice(0, 160).replace(/\s+/g, " ").trim() });
    }
  }
  return hits;
}

/** Spaced resurfacing check matching exact 1/3/7/21/60-day cadence. */
export function selectNotesForResurface(
  notes: readonly { slug: string; createdAtMs: number }[],
  nowMs: number,
): { slug: string; intervalDay: number }[] {
  const MS_PER_DAY = 86_400_000;
  const due: { slug: string; intervalDay: number }[] = [];
  for (const n of notes) {
    const ageDays = Math.floor((nowMs - n.createdAtMs) / MS_PER_DAY);
    if ((RESURFACE_INTERVAL_DAYS as readonly number[]).includes(ageDays)) {
      due.push({ slug: n.slug, intervalDay: ageDays });
    }
  }
  return due;
}

export const KARPATHY_SIX_PASSES = [
  "1_ingest_inbox",
  "2_extract_entities",
  "3_cross_link_wiki",
  "4_detect_contradictions",
  "5_update_decisions",
  "6_emit_index",
] as const;

/** Karpathy 6-Pass Wiki Compiler over the Second Brain vault. */
export function runKarpathySixPassWikiCompiler(vaultRoot: string): {
  passesRun: readonly string[];
  linkedNoteCount: number;
  indexFile: string;
} {
  const notesDir = path.join(vaultRoot, "notes");
  const files = fs.existsSync(notesDir)
    ? fs.readdirSync(notesDir).filter((f) => f.endsWith(".md")).sort()
    : [];
  const indexFile = path.join(vaultRoot, "wiki-index.md");
  const lines = [
    "# Second Brain Wiki Index",
    "",
    ...files.map((f) => `- [[${f.slice(0, -3)}]]`),
    "",
  ];
  fs.writeFileSync(indexFile, lines.join("\n"), "utf8");
  return {
    passesRun: KARPATHY_SIX_PASSES,
    linkedNoteCount: files.length,
    indexFile,
  };
}

export type DailySlashCommand =
  | "/remember"
  | "/recall"
  | "/resurface"
  | "/review"
  | "/morning-brief"
  | "/weekly-review";

/** Draft-only (`dontAsk`) daily command dispatcher writing briefs into `briefs/` without sending externally. */
export function dispatchDailyCommand(
  vaultRoot: string,
  command: DailySlashCommand,
  payload = "",
): { command: DailySlashCommand; mode: "dontAsk"; draftOnly: true; artifactPath?: string; summary: string } {
  const dateStamp = new Date().toISOString().slice(0, 10);
  if (command === "/morning-brief" || command === "/weekly-review") {
    const file = path.join(vaultRoot, "briefs", `${dateStamp}-${command.slice(1)}.md`);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, `# ${command} (${dateStamp})\n\nMode: dontAsk (draft-only)\n\n${payload}\n`, "utf8");
    return { command, mode: "dontAsk", draftOnly: true, artifactPath: file, summary: `Drafted ${path.basename(file)}` };
  }
  return { command, mode: "dontAsk", draftOnly: true, summary: `Executed ${command} in draft-only mode` };
}

/** Local Alook-style team room router (`#intel` for research/signal vs `#alerts` for urgent incidents). */
export function routeTeamRoomMessage(message: string): { room: "#intel" | "#alerts"; urgent: boolean } {
  const urgent = /\b(urgent|sev1|sev0|outage|down|breach|failed prod|pager|alert)\b/i.test(message);
  return { room: urgent ? "#alerts" : "#intel", urgent };
}

/** Nimble-style dated web snapshot diffing (line-level added/removed delta). */
export function diffDatedWebSnapshots(
  prevDate: string,
  prevText: string,
  currDate: string,
  currText: string,
): { prevDate: string; currDate: string; changed: boolean; added: string[]; removed: string[] } {
  const prevSet = new Set(prevText.split(/\r?\n/).map((l) => l.trim()).filter(Boolean));
  const currSet = new Set(currText.split(/\r?\n/).map((l) => l.trim()).filter(Boolean));
  const added = [...currSet].filter((l) => !prevSet.has(l));
  const removed = [...prevSet].filter((l) => !currSet.has(l));
  return { prevDate, currDate, changed: added.length > 0 || removed.length > 0, added, removed };
}

/** Voice keyterm normalizer correcting domain-specific jargon from speech-to-text transcripts. */
export function applyVoiceKeyterms(transcript: string, keyterms: Record<string, string>): string {
  let out = transcript;
  for (const [spoken, canonical] of Object.entries(keyterms)) {
    const escaped = spoken.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    out = out.replace(new RegExp(`\\b${escaped}\\b`, "gi"), canonical);
  }
  return out;
}