import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { evaluateAmbiguityGate, runTurn0Bootstrap } from "../src/bootstrap";

describe("P2-03 Turn-0 Bootstrap & Kimi K3 Ambiguity Gate", () => {
  test("bootstraps workspace fast with git, compilers, RAM and schema probes", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-boot-"));
    try {
      fs.writeFileSync(path.join(ws, "openapi.json"), "{}");
      fs.writeFileSync(path.join(ws, "init.sql"), "SELECT 1;");
      const snap = runTurn0Bootstrap(ws);
      expect(snap.elapsedMs).toBeLessThan(200);
      expect(snap.ramGb).toBeGreaterThanOrEqual(1);
      expect(snap.recommendedMakeJobs).toBeGreaterThanOrEqual(1);
      expect(snap.schemas.hasOpenApi).toBeTrue();
      expect(snap.schemas.sqlFiles).toContain("init.sql");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("pauses for at most 2 MCQ questions on high-impact ambiguities and writes assumptions.md for minor ones", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-ambig-"));
    try {
      const verdict = evaluateAmbiguityGate(ws, [
        {
          id: "db",
          question: "Which database engine?",
          options: ["SQLite", "Postgres"],
          impact: "high",
          defaultAssumption: "SQLite",
        },
        {
          id: "auth",
          question: "Which auth mode?",
          options: ["JWT", "Session cookie"],
          impact: "high",
          defaultAssumption: "Session cookie",
        },
        {
          id: "extra",
          question: "Third high-impact question (must be capped at 2)",
          options: ["A", "B"],
          impact: "high",
          defaultAssumption: "A",
        },
        {
          id: "port",
          question: "Default dev port?",
          options: ["3000", "8080"],
          impact: "minor_reversible",
          defaultAssumption: "Use port 3000",
        },
      ]);
      expect(verdict.status).toBe("pause_for_clarification");
      if (verdict.status === "pause_for_clarification") {
        expect(verdict.questions).toHaveLength(2);
      }
      expect(fs.existsSync(path.join(ws, "assumptions.md"))).toBeTrue();
      expect(fs.readFileSync(path.join(ws, "assumptions.md"), "utf8")).toContain("Use port 3000");

      const minorOnly = evaluateAmbiguityGate(ws, [
        {
          id: "indent",
          question: "2 or 4 spaces?",
          options: ["2", "4"],
          impact: "minor_reversible",
          defaultAssumption: "2 spaces",
        },
      ]);
      expect(minorOnly.status).toBe("proceed");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});