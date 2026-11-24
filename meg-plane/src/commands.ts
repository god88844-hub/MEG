// P2-08: Magnitude Router (M0-M5), pstack /mode with >10-file blast-radius pause,
// 7 slash commands (/verify, /commit, /review, /refactor, /spec, /garden, /clear),
// and .meg/backlog.md ticket drop (prevents scope poisoning).

import fs from "node:fs";
import path from "node:path";

export type MagnitudeLevel = "M0" | "M1" | "M2" | "M3" | "M4" | "M5";

export interface MagnitudeSpec {
  level: MagnitudeLevel;
  label: string;
  maxFiles: number;
  maxTurns: number;
  maxBudgetUsd: number;
  allowSwarm: boolean;
}

export const MAGNITUDE_TABLE: Record<MagnitudeLevel, MagnitudeSpec> = {
  M0: { level: "M0", label: "Nano", maxFiles: 1, maxTurns: 3, maxBudgetUsd: 0.05, allowSwarm: false },
  M1: { level: "M1", label: "Micro", maxFiles: 3, maxTurns: 10, maxBudgetUsd: 0.25, allowSwarm: false },
  M2: { level: "M2", label: "Small", maxFiles: 8, maxTurns: 25, maxBudgetUsd: 1.00, allowSwarm: false },
  M3: { level: "M3", label: "Medium", maxFiles: 20, maxTurns: 60, maxBudgetUsd: 3.00, allowSwarm: true },
  M4: { level: "M4", label: "Large", maxFiles: 60, maxTurns: 120, maxBudgetUsd: 8.00, allowSwarm: true },
  M5: { level: "M5", label: "Mega-Engine", maxFiles: 500, maxTurns: 250, maxBudgetUsd: 25.00, allowSwarm: true },
};

export function routeMagnitude(estimatedFiles: number, estimatedComplexity: "trivial" | "normal" | "complex" | "mega" = "normal"): MagnitudeSpec {
  if (estimatedComplexity === "mega" || estimatedFiles > 60) return MAGNITUDE_TABLE.M5;
  if (estimatedFiles <= 1 && estimatedComplexity === "trivial") return MAGNITUDE_TABLE.M0;
  if (estimatedFiles <= 3 && estimatedComplexity !== "complex") return MAGNITUDE_TABLE.M1;
  if (estimatedFiles <= 8 && estimatedComplexity !== "complex") return MAGNITUDE_TABLE.M2;
  if (estimatedFiles <= 20) return MAGNITUDE_TABLE.M3;
  return MAGNITUDE_TABLE.M4;
}

export type PstackMode = "feature" | "refactor" | "bug-hunt" | "migration";
export const PSTACK_MODES: readonly PstackMode[] = ["feature", "refactor", "bug-hunt", "migration"];
export const BLAST_RADIUS_PAUSE_FILES = 10;

export interface ModeActivationResult {
  mode: PstackMode;
  blastRadiusFiles: number;
  pausedForApproval: boolean;
  message: string;
}

export function evaluateModeBlastRadius(mode: PstackMode, plannedFiles: readonly string[]): ModeActivationResult {
  const count = plannedFiles.length;
  const pausedForApproval = count > BLAST_RADIUS_PAUSE_FILES;
  return {
    mode,
    blastRadiusFiles: count,
    pausedForApproval,
    message: pausedForApproval
      ? `Mode /mode ${mode} paused: blast radius (${count} files) exceeds >${BLAST_RADIUS_PAUSE_FILES} file threshold; operator confirmation required.`
      : `Mode /mode ${mode} active (${count} planned file(s)).`,
  };
}

export const SLASH_COMMANDS = [
  "/verify",
  "/commit",
  "/review",
  "/refactor",
  "/spec",
  "/garden",
  "/clear",
  "/mode",
] as const;

export interface CommandContext {
  workspace: string;
  sessionEditedFiles: string[];
  currentMode: PstackMode;
}

export interface CommandResult {
  command: string;
  ok: boolean;
  action: string;
  details: Record<string, unknown>;
}

/** Drop an out-of-scope idea into `.meg/backlog.md` to prevent scope poisoning. */
export function dropBacklogTicket(workspace: string, title: string, reason = "deferred out-of-scope item"): string {
  const cleanTitle = title.trim();
  if (!cleanTitle) throw new Error("Backlog ticket requires a non-empty title");
  const file = path.join(workspace, ".meg", "backlog.md");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  if (!fs.existsSync(file)) {
    fs.writeFileSync(file, "# MEG Backlog\n\n", "utf8");
  }
  const stamp = new Date().toISOString().slice(0, 10);
  const line = `- [ ] [${stamp}] ${cleanTitle} -- _${reason}_\n`;
  fs.appendFileSync(file, line, "utf8");
  return file;
}

/** Dispatch the 7 v1.2 slash commands + `/mode`. */
export function dispatchSlashCommand(rawInput: string, ctx: CommandContext): CommandResult {
  const trimmed = rawInput.trim();
  const [cmd, ...rest] = trimmed.split(/\s+/);
  const arg = rest.join(" ").trim();

  switch (cmd) {
    case "/verify":
      return {
        command: "/verify",
        ok: true,
        action: "run_phase_verification",
        details: { script: process.platform === "win32" ? "scripts/check.ps1" : "scripts/check.sh" },
      };

    case "/commit": {
      if (ctx.sessionEditedFiles.length === 0) {
        return {
          command: "/commit",
          ok: false,
          action: "noop_no_session_edits",
          details: { error: "Refusing /commit: no session-edited files recorded" },
        };
      }
      const uniqueFiles = [...new Set(ctx.sessionEditedFiles)];
      return {
        command: "/commit",
        ok: true,
        action: "stage_session_files_only",
        details: {
          files: uniqueFiles,
          message: arg || "chore: commit session-edited changes",
          gitAddArgv: ["git", "add", "--", ...uniqueFiles],
        },
      };
    }

    case "/review":
      return {
        command: "/review",
        ok: true,
        action: "launch_cold_cross_model_review",
        details: { files: [...new Set(ctx.sessionEditedFiles)], mode: "read-only" },
      };

    case "/refactor":
      return {
        command: "/refactor",
        ok: true,
        action: "run_refactor_analyzers",
        details: {
          tools: ["jscpd", "knip", "ast-grep"],
          target: arg || ".",
        },
      };

    case "/spec":
      return {
        command: "/spec",
        ok: true,
        action: "draft_preflight_contract",
        details: { prompt: arg },
      };

    case "/garden":
      return {
        command: "/garden",
        ok: true,
        action: "garden_rules_and_backlog",
        details: { backlogFile: path.join(ctx.workspace, ".meg", "backlog.md") },
      };

    case "/clear":
      return {
        command: "/clear",
        ok: true,
        action: "reset_active_context_preserve_ledger",
        details: { preserveLedger: true },
      };

    case "/mode": {
      const requested = arg as PstackMode;
      if (!PSTACK_MODES.includes(requested)) {
        return {
          command: "/mode",
          ok: false,
          action: "invalid_mode",
          details: { allowed: PSTACK_MODES },
        };
      }
      const res = evaluateModeBlastRadius(requested, ctx.sessionEditedFiles);
      return {
        command: "/mode",
        ok: true,
        action: res.pausedForApproval ? "pause_blast_radius" : "switch_mode",
        details: res as unknown as Record<string, unknown>,
      };
    }

    default:
      return {
        command: cmd ?? "",
        ok: false,
        action: "unknown_command",
        details: { available: SLASH_COMMANDS },
      };
  }
}