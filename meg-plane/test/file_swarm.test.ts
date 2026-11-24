import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  MAX_SWARM_LESSONS_LINES,
  appendSwarmLesson,
  initFileSwarm,
  validateFileSwarm,
  writeSwarmOutput,
} from "../src/file_swarm";

describe("P5-03 7-Rule File Swarm", () => {
  test("rejects overlapping task scopes, missing END sentinel, uncited FINAL.md paragraphs & enforces <=30-line LESSONS.md", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p503-"));
    try {
      expect(() =>
        initFileSwarm(ws, [
          { id: "T1", title: "A", owner: "w1", ownedFiles: ["src/shared.ts"], objective: "a" },
          { id: "T2", title: "B", owner: "w2", ownedFiles: ["src/shared.ts"], objective: "b" },
        ]),
      ).toThrow("FILE_SWARM_SCOPE_CONFLICT");

      initFileSwarm(ws, [
        { id: "T1", title: "Router", owner: "w1", ownedFiles: ["src/router.ts"], objective: "router" },
        { id: "T2", title: "Tests", owner: "w2", ownedFiles: ["test/router.test.ts"], objective: "tests" },
      ]);

      // Missing END sentinel fails validation
      writeSwarmOutput(ws, "T1", "Implemented router.", false);
      writeSwarmOutput(ws, "T2", "Added unit tests.", true);
      fs.writeFileSync(path.join(ws, "FINAL.md"), "# Final\n\nUncited claim without provenance.\n");

      const bad = validateFileSwarm(ws, ["T1", "T2"]);
      expect(bad.ok).toBeFalse();
      expect(bad.errors.some((e) => e.includes("END sentinel"))).toBeTrue();
      expect(bad.errors.some((e) => e.includes("[T#] provenance"))).toBeTrue();

      // Fix sentinel & add [T1]/[T2] citations + fill 40 lessons (must cap at <=30)
      writeSwarmOutput(ws, "T1", "Implemented router.", true);
      fs.writeFileSync(
        path.join(ws, "FINAL.md"),
        "# Final\n\nRouter hardened [T1].\n\nTests verified [T2].\n",
      );
      for (let i = 1; i <= 40; i++) {
        appendSwarmLesson(ws, `Lesson ${i}`);
      }
      const lineCount = fs
        .readFileSync(path.join(ws, "LESSONS.md"), "utf8")
        .split(/\r?\n/)
        .filter((l) => l.trim().length > 0).length;
      expect(lineCount).toBeLessThanOrEqual(MAX_SWARM_LESSONS_LINES);

      expect(validateFileSwarm(ws, ["T1", "T2"]).ok).toBeTrue();
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});