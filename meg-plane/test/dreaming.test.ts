import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  RuleWorkspace,
  appendLearning,
  promoteStandingRules,
  runOfflineDreamingPass,
  steerMemoryForGear,
} from "../src/dreaming";

describe("P3-02 Model-Tier Steering, ACE Librarian & Offline Dreaming", () => {
  test("keeps c_0 + compact index separated, gardens obsolete OPTIMIZATION rules, and steers by gear", () => {
    const rw = new RuleWorkspace("CORE: Always verify before claiming done.");
    rw.registerRule({
      id: "R1",
      category: "INSURANCE",
      summary: "Never leak secrets",
      body: "Full secret policy body...",
      verifiedUses: 12,
      obsolete: true, // INSURANCE must NEVER be pruned even if flagged
    });
    rw.registerRule({
      id: "R2",
      category: "OPTIMIZATION",
      summary: "Legacy JSON wrapper hint",
      body: "Old prompt trick...",
      verifiedUses: 1,
      obsolete: true,
    });

    const compact = rw.renderCompactPrompt();
    expect(compact).toContain("CORE:");
    expect(compact).toContain("R1");
    expect(compact).not.toContain("Full secret policy body...");
    expect(rw.fetchRuleBody("R1")).toContain("Full secret policy");

    const gardened = rw.gardenRules();
    expect(gardened.prunedIds).toEqual(["R2"]);
    expect(gardened.kept.map((r) => r.id)).toEqual(["R1"]);

    expect(steerMemoryForGear(3).mode).toBe("open_salience");
    expect(steerMemoryForGear(1).mandatoryPreHooks).toEqual(["Read-Memory", "Test-Theory"]);
    expect(steerMemoryForGear(2).mandatoryPreHooks).toEqual(["Read-Memory", "Test-Theory"]);
  });

  test("ACE LEARNINGS.md caps promoted rules at <=30 and Offline Dreaming builds linked wiki/", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p302-"));
    try {
      for (let i = 1; i <= 40; i++) {
        appendLearning(ws, {
          category: i <= 5 ? "INSURANCE" : "OPTIMIZATION",
          rule: `Rule number ${i}`,
          evidence: `trace-${i}`,
        });
      }
      const promoted = promoteStandingRules(ws);
      expect(promoted).toHaveLength(30);
      expect(promoted[0]).toContain("[INSURANCE]");

      const report = runOfflineDreamingPass(
        ws,
        ["cargo test passed with 399 tests and autocrlf=false fixed side-git"],
        [
          {
            slug: "side-git-crlf",
            title: "Side-Git CRLF Fix",
            claim: "Always pass -c core.autocrlf=false on Windows.",
            expectedEvidence: "autocrlf=false",
            links: ["windows-shell"],
          },
          {
            slug: "hallucinated-note",
            title: "Unverified Claim",
            claim: "Claim not present in transcript.",
            expectedEvidence: "nonexistent-marker-999",
          },
        ],
      );
      expect(report.confirmed).toEqual(["side-git-crlf"]);
      expect(report.contradicted).toEqual(["hallucinated-note"]);
      expect(fs.readFileSync(report.wikiIndexFile, "utf8")).toContain("[[side-git-crlf]]");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});