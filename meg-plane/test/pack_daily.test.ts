import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { loadSkillFromDir } from "../src/skills";
import {
  KARPATHY_SIX_PASSES,
  SECOND_BRAIN_DIRS,
  applyVoiceKeyterms,
  diffDatedWebSnapshots,
  dispatchDailyCommand,
  initSecondBrainVault,
  recallNotes,
  rememberNote,
  routeTeamRoomMessage,
  runKarpathySixPassWikiCompiler,
  selectNotesForResurface,
} from "../src/pack_daily";

describe("P6-07 Pack 6 -- Daily Usage, Second Brain & Team Rooms", () => {
  test("initializes Obsidian Second Brain, runs Karpathy 6-Pass compiler, 1/3/7/21/60d resurface, dontAsk briefs, Alook rooms, Nimble diff & voice keyterms", () => {
    const skill = loadSkillFromDir(path.resolve(import.meta.dir, "../skills/pack-daily"));
    expect(skill.metadata.name).toBe("pack-daily");

    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p607-"));
    try {
      const vault = initSecondBrainVault(ws);
      for (const d of SECOND_BRAIN_DIRS) expect(fs.existsSync(path.join(ws, d))).toBeTrue();
      expect(fs.existsSync(vault.decisionsFile)).toBeTrue();

      const now = Date.now();
      const day = 86_400_000;
      rememberNote(ws, { slug: "arch-notes", title: "MEG Architecture", body: "Ring-0 Rust + Bun plane", createdAtMs: now - 7 * day, tags: ["meg"] });
      expect(recallNotes(ws, "Ring-0")).toHaveLength(1);

      const due = selectNotesForResurface(
        [
          { slug: "d1", createdAtMs: now - 1 * day },
          { slug: "d2", createdAtMs: now - 2 * day }, // not due
          { slug: "d7", createdAtMs: now - 7 * day },
          { slug: "d60", createdAtMs: now - 60 * day },
        ],
        now,
      );
      expect(due.map((d) => d.intervalDay)).toEqual([1, 7, 60]);

      const compiled = runKarpathySixPassWikiCompiler(ws);
      expect(compiled.passesRun).toEqual(KARPATHY_SIX_PASSES);
      expect(fs.readFileSync(compiled.indexFile, "utf8")).toContain("[[arch-notes]]");

      const brief = dispatchDailyCommand(ws, "/morning-brief", "Focus: Phase 7");
      expect(brief.mode).toBe("dontAsk");
      expect(brief.draftOnly).toBeTrue();
      expect(fs.existsSync(brief.artifactPath!)).toBeTrue();

      expect(routeTeamRoomMessage("SEV1 production outage on API").room).toBe("#alerts");
      expect(routeTeamRoomMessage("New paper on KV-cache compaction").room).toBe("#intel");

      const diff = diffDatedWebSnapshots("2026-11-20", "Price: ₹999\nPlan: Pro", "2026-11-24", "Price: ₹1199\nPlan: Pro");
      expect(diff.changed).toBeTrue();
      expect(diff.added).toEqual(["Price: ₹1199"]);
      expect(diff.removed).toEqual(["Price: ₹999"]);

      expect(applyVoiceKeyterms("open meg tui and run pie test", { "meg tui": "meg-tui", "pie test": "pytest" })).toBe(
        "open meg-tui and run pytest",
      );
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});