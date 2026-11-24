// Turn-0 Bootstrap (<50ms target) & Kimi K3 Ambiguity Gate (P2-03).

import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export interface BootstrapSnapshot {
  elapsedMs: number;
  platform: NodeJS.Platform;
  arch: string;
  shell: string;
  ramGb: number;
  cpuCount: number;
  recommendedMakeJobs: number;
  git: { isRepo: boolean; headRef: string | null };
  compilers: Record<string, boolean>;
  schemas: {
    hasPrisma: boolean;
    hasMigrations: boolean;
    hasOpenApi: boolean;
    sqlFiles: string[];
  };
}

function fastCommandExists(name: string, pathEnv: string): boolean {
  const dirs = pathEnv.split(path.delimiter).filter(Boolean);
  const exts = process.platform === "win32" ? [".exe", ".cmd", ".bat", ""] : [""];
  for (const dir of dirs) {
    for (const ext of exts) {
      if (fs.existsSync(path.join(dir, `${name}${ext}`))) return true;
    }
  }
  return false;
}

function probeGitHead(workspace: string): { isRepo: boolean; headRef: string | null } {
  const dotGit = path.join(workspace, ".git");
  if (!fs.existsSync(dotGit)) return { isRepo: false, headRef: null };
  try {
    let gitDir = dotGit;
    if (fs.statSync(dotGit).isFile()) {
      const ptr = fs.readFileSync(dotGit, "utf8").trim();
      const match = ptr.match(/^gitdir:\s*(.+)$/i);
      if (match?.[1]) gitDir = path.resolve(workspace, match[1].trim());
    }
    const headFile = path.join(gitDir, "HEAD");
    if (!fs.existsSync(headFile)) return { isRepo: true, headRef: null };
    const head = fs.readFileSync(headFile, "utf8").trim();
    const ref = head.startsWith("ref:") ? head.slice(4).trim() : head.slice(0, 12);
    return { isRepo: true, headRef: ref };
  } catch {
    return { isRepo: true, headRef: null };
  }
}

function probeSchemas(workspace: string) {
  const hasPrisma = fs.existsSync(path.join(workspace, "prisma", "schema.prisma"));
  const hasMigrations = fs.existsSync(path.join(workspace, "migrations")) ||
    fs.existsSync(path.join(workspace, "prisma", "migrations"));
  const hasOpenApi =
    fs.existsSync(path.join(workspace, "openapi.yaml")) ||
    fs.existsSync(path.join(workspace, "openapi.yml")) ||
    fs.existsSync(path.join(workspace, "openapi.json")) ||
    fs.existsSync(path.join(workspace, "swagger.json"));
  const sqlFiles: string[] = [];
  try {
    for (const entry of fs.readdirSync(workspace, { withFileTypes: true })) {
      if (entry.isFile() && entry.name.toLowerCase().endsWith(".sql")) {
        sqlFiles.push(entry.name);
      }
    }
  } catch {
    // ignore unreadable dir
  }
  return { hasPrisma, hasMigrations, hasOpenApi, sqlFiles };
}

/** Fast (<50ms) Turn-0 workspace/environment probe with zero subprocess forks. */
export function runTurn0Bootstrap(workspace: string): BootstrapSnapshot {
  const started = performance.now();
  const pathEnv = process.env.PATH ?? process.env.Path ?? "";
  const ramGb = Math.max(1, Math.round(os.totalmem() / (1024 ** 3)));
  const shell =
    process.platform === "win32"
      ? process.env.ComSpec ?? "powershell.exe"
      : process.env.SHELL ?? "/bin/sh";

  const compilers: Record<string, boolean> = {};
  for (const bin of ["cargo", "rustc", "bun", "node", "python", "git", "gcc"]) {
    compilers[bin] = fastCommandExists(bin, pathEnv);
  }

  const git = probeGitHead(workspace);
  const schemas = probeSchemas(workspace);
  const elapsedMs = Math.round((performance.now() - started) * 100) / 100;

  return {
    elapsedMs,
    platform: process.platform,
    arch: process.arch,
    shell,
    ramGb,
    cpuCount: os.cpus().length,
    recommendedMakeJobs: Math.max(1, Math.floor(ramGb / 2)),
    git,
    compilers,
    schemas,
  };
}

export interface AmbiguityCandidate {
  id: string;
  question: string;
  options: string[];
  impact: "high" | "minor_reversible";
  defaultAssumption: string;
}

export type AmbiguityVerdict =
  | {
      status: "pause_for_clarification";
      questions: { id: string; question: string; options: string[] }[];
      assumptionsFile?: string;
    }
  | {
      status: "proceed";
      assumptionsFile?: string;
      recordedAssumptions: string[];
    };

/**
 * Kimi K3 Turn-0 Ambiguity Gate:
 * - High-impact ambiguities pause for <= 2 concise MCQ questions.
 * - Minor reversible assumptions are recorded to non-blocking `assumptions.md`.
 */
export function evaluateAmbiguityGate(
  workspace: string,
  candidates: readonly AmbiguityCandidate[],
): AmbiguityVerdict {
  const high = candidates.filter((c) => c.impact === "high");
  const minor = candidates.filter((c) => c.impact === "minor_reversible");

  let assumptionsFile: string | undefined;
  const recordedAssumptions: string[] = [];

  if (minor.length > 0) {
    assumptionsFile = path.join(workspace, "assumptions.md");
    const lines = [
      "# Assumptions (Reversible Defaults)",
      "",
      ...minor.map((m) => {
        const entry = `- **${m.id}**: ${m.defaultAssumption} *(question: ${m.question})*`;
        recordedAssumptions.push(`${m.id}: ${m.defaultAssumption}`);
        return entry;
      }),
      "",
    ];
    fs.writeFileSync(assumptionsFile, lines.join("\n"), "utf8");
  }

  if (high.length > 0) {
    return {
      status: "pause_for_clarification",
      questions: high.slice(0, 2).map((q) => ({
        id: q.id,
        question: q.question,
        options: q.options.slice(0, 4),
      })),
      assumptionsFile,
    };
  }

  return {
    status: "proceed",
    assumptionsFile,
    recordedAssumptions,
  };
}