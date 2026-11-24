// P5-03: 7-Rule File Swarm:
// 1. BOARD.md (task status board)
// 2. Pairwise scope-tightened tasks/T[n].md (zero file ownership overlap)
// 3. outputs/T[n].md requiring final-line `END` sentinel
// 4. checks/CONFLICTS.md conflict register
// 5. FINAL.md requiring `[T#]` paragraph provenance on every prose paragraph
// 6. LESSONS.md capped at <=30 lines
// 7. Deterministic validator enforcing all rules before swarm merge

import fs from "node:fs";
import path from "node:path";

export const MAX_SWARM_LESSONS_LINES = 30;

export interface SwarmTaskSpec {
  id: string; // e.g. "T1", "T2"
  title: string;
  owner: string;
  ownedFiles: string[];
  objective: string;
  status?: "pending" | "in_progress" | "done" | "blocked";
}

/** Pairwise scope-tightening check: throws if any two tasks claim the same file. */
export function verifyPairwiseDisjointScopes(tasks: readonly SwarmTaskSpec[]): { ok: boolean; overlaps: string[] } {
  const overlaps: string[] = [];
  for (let i = 0; i < tasks.length; i++) {
    const a = tasks[i]!;
    const setA = new Set(a.ownedFiles.map((f) => f.replace(/\\/g, "/")));
    for (let j = i + 1; j < tasks.length; j++) {
      const b = tasks[j]!;
      for (const f of b.ownedFiles) {
        const norm = f.replace(/\\/g, "/");
        if (setA.has(norm)) {
          overlaps.push(`${a.id} <-> ${b.id} overlap on '${norm}'`);
        }
      }
    }
  }
  return { ok: overlaps.length === 0, overlaps };
}

/** Initialize the 7-Rule File Swarm workspace layout (`BOARD.md`, `tasks/T[n].md`, `checks/CONFLICTS.md`, `LESSONS.md`). */
export function initFileSwarm(swarmDir: string, tasks: readonly SwarmTaskSpec[]): void {
  const scope = verifyPairwiseDisjointScopes(tasks);
  if (!scope.ok) {
    throw new Error(`FILE_SWARM_SCOPE_CONFLICT: ${scope.overlaps.join("; ")}`);
  }

  fs.mkdirSync(path.join(swarmDir, "tasks"), { recursive: true });
  fs.mkdirSync(path.join(swarmDir, "outputs"), { recursive: true });
  fs.mkdirSync(path.join(swarmDir, "checks"), { recursive: true });

  const boardLines = [
    "# Swarm Board",
    "",
    "| Task | Owner | Status | Owned Files |",
    "| :--- | :--- | :--- | :--- |",
    ...tasks.map(
      (t) =>
        `| ${t.id} | ${t.owner} | ${t.status ?? "pending"} | ${t.ownedFiles.join(", ")} |`,
    ),
    "",
  ];
  fs.writeFileSync(path.join(swarmDir, "BOARD.md"), boardLines.join("\n"), "utf8");

  for (const t of tasks) {
    if (!/^T\d+$/.test(t.id)) {
      throw new Error(`Swarm task id must match T[n] (got '${t.id}')`);
    }
    const taskMd = [
      `# ${t.id}: ${t.title}`,
      ``,
      `- Owner: ${t.owner}`,
      `- Scope (exclusive): ${t.ownedFiles.join(", ")}`,
      ``,
      `## Objective`,
      t.objective,
      ``,
    ].join("\n");
    fs.writeFileSync(path.join(swarmDir, "tasks", `${t.id}.md`), taskMd, "utf8");
  }

  fs.writeFileSync(path.join(swarmDir, "checks", "CONFLICTS.md"), "# Conflicts\n\n(none)\n", "utf8");
  if (!fs.existsSync(path.join(swarmDir, "LESSONS.md"))) {
    fs.writeFileSync(path.join(swarmDir, "LESSONS.md"), "# Lessons\n\n", "utf8");
  }
}

/** Write `outputs/T[n].md` ensuring the mandatory final-line `END` sentinel. */
export function writeSwarmOutput(swarmDir: string, taskId: string, body: string, includeEndSentinel = true): string {
  const file = path.join(swarmDir, "outputs", `${taskId}.md`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const trimmed = body.trimEnd();
  const content = includeEndSentinel ? `${trimmed}\nEND\n` : `${trimmed}\n`;
  fs.writeFileSync(file, content, "utf8");
  return file;
}

export function hasEndSentinel(raw: string): boolean {
  const nonEmpty = raw
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  return nonEmpty.at(-1) === "END";
}

/** Verify every prose paragraph in `FINAL.md` carries at least one `[T#]` provenance citation. */
export function checkFinalProvenance(finalMd: string): { ok: boolean; uncitedParagraphs: number[] } {
  const blocks = finalMd
    .split(/\r?\n\s*\r?\n/)
    .map((b) => b.trim())
    .filter((b) => b.length > 0 && !b.startsWith("#"));
  const uncitedParagraphs: number[] = [];
  for (let i = 0; i < blocks.length; i++) {
    if (!/\[T\d+\]/.test(blocks[i]!)) {
      uncitedParagraphs.push(i + 1);
    }
  }
  return { ok: uncitedParagraphs.length === 0, uncitedParagraphs };
}

/** Append a lesson to `LESSONS.md`, enforcing the hard `<=30`-line cap. */
export function appendSwarmLesson(swarmDir: string, lesson: string): number {
  const file = path.join(swarmDir, "LESSONS.md");
  const existing = fs.existsSync(file)
    ? fs
        .readFileSync(file, "utf8")
        .split(/\r?\n/)
        .filter((l) => l.trim().length > 0)
    : ["# Lessons"];
  existing.push(`- ${lesson.trim()}`);
  const capped = existing.slice(0, MAX_SWARM_LESSONS_LINES);
  fs.writeFileSync(file, capped.join("\n") + "\n", "utf8");
  return capped.length;
}

export interface FileSwarmValidationReport {
  ok: boolean;
  errors: string[];
}

/** Validate all 7 File Swarm artifacts in `swarmDir`. */
export function validateFileSwarm(swarmDir: string, taskIds: readonly string[]): FileSwarmValidationReport {
  const errors: string[] = [];

  if (!fs.existsSync(path.join(swarmDir, "BOARD.md"))) errors.push("Missing BOARD.md");
  if (!fs.existsSync(path.join(swarmDir, "checks", "CONFLICTS.md"))) errors.push("Missing checks/CONFLICTS.md");

  for (const id of taskIds) {
    if (!fs.existsSync(path.join(swarmDir, "tasks", `${id}.md`))) {
      errors.push(`Missing tasks/${id}.md`);
    }
    const outFile = path.join(swarmDir, "outputs", `${id}.md`);
    if (!fs.existsSync(outFile)) {
      errors.push(`Missing outputs/${id}.md`);
    } else if (!hasEndSentinel(fs.readFileSync(outFile, "utf8"))) {
      errors.push(`outputs/${id}.md missing final-line END sentinel`);
    }
  }

  const finalFile = path.join(swarmDir, "FINAL.md");
  if (!fs.existsSync(finalFile)) {
    errors.push("Missing FINAL.md");
  } else {
    const prov = checkFinalProvenance(fs.readFileSync(finalFile, "utf8"));
    if (!prov.ok) {
      errors.push(`FINAL.md paragraph(s) missing [T#] provenance: ${prov.uncitedParagraphs.join(", ")}`);
    }
  }

  const lessonsFile = path.join(swarmDir, "LESSONS.md");
  if (!fs.existsSync(lessonsFile)) {
    errors.push("Missing LESSONS.md");
  } else {
    const lines = fs
      .readFileSync(lessonsFile, "utf8")
      .split(/\r?\n/)
      .filter((l) => l.trim().length > 0);
    if (lines.length > MAX_SWARM_LESSONS_LINES) {
      errors.push(`LESSONS.md exceeds ${MAX_SWARM_LESSONS_LINES} lines (${lines.length})`);
    }
  }

  return { ok: errors.length === 0, errors };
}