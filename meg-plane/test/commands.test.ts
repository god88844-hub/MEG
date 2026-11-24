import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  MAGNITUDE_TABLE,
  dispatchSlashCommand,
  dropBacklogTicket,
  evaluateModeBlastRadius,
  routeMagnitude,
} from "../src/commands";

describe("P2-08 Magnitude Router, /mode Blast Radius & 7 Slash Commands", () => {
  test("routes M0 Nano (1 file, <=3 turns, $0.05) through M5 Mega-Engine (250 turns)", () => {
    const m0 = routeMagnitude(1, "trivial");
    expect(m0).toEqual(MAGNITUDE_TABLE.M0);
    expect(m0.maxTurns).toBe(3);
    expect(m0.maxBudgetUsd).toBe(0.05);

    const m5 = routeMagnitude(80, "mega");
    expect(m5.level).toBe("M5");
    expect(m5.maxTurns).toBe(250);
    expect(m5.allowSwarm).toBeTrue();
  });

  test("pstack /mode pauses when blast radius exceeds 10 files", () => {
    const ok = evaluateModeBlastRadius("refactor", ["a.ts", "b.ts"]);
    expect(ok.pausedForApproval).toBeFalse();

    const files11 = Array.from({ length: 11 }, (_, i) => `src/f${i}.ts`);
    const paused = evaluateModeBlastRadius("migration", files11);
    expect(paused.pausedForApproval).toBeTrue();
    expect(paused.message).toContain(">10");
  });

  test("handles all 7 slash commands (/commit scopes to session-edited files, /refactor lists jscpd+knip+ast-grep) and drops backlog tickets", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p208-"));
    try {
      const baseCtx = { workspace: ws, sessionEditedFiles: ["src/a.ts", "src/b.ts"], currentMode: "feature" as const };

      expect(dispatchSlashCommand("/verify", baseCtx).ok).toBeTrue();
      const commit = dispatchSlashCommand("/commit fix edge case", baseCtx);
      expect(commit.ok).toBeTrue();
      expect(commit.details.files).toEqual(["src/a.ts", "src/b.ts"]);
      expect(dispatchSlashCommand("/commit", { ...baseCtx, sessionEditedFiles: [] }).ok).toBeFalse();

      expect(dispatchSlashCommand("/review", baseCtx).action).toBe("launch_cold_cross_model_review");
      const ref = dispatchSlashCommand("/refactor src", baseCtx);
      expect(ref.details.tools).toEqual(["jscpd", "knip", "ast-grep"]);
      expect(dispatchSlashCommand("/spec add cache", baseCtx).ok).toBeTrue();
      expect(dispatchSlashCommand("/garden", baseCtx).ok).toBeTrue();
      expect(dispatchSlashCommand("/clear", baseCtx).details.preserveLedger).toBeTrue();

      const ticketFile = dropBacklogTicket(ws, "Investigate flaky parser", "out of scope for P2-08");
      const raw = fs.readFileSync(ticketFile, "utf8");
      expect(raw).toContain("Investigate flaky parser");
      expect(raw).toContain("out of scope for P2-08");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});