import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { loadSkillFromDir } from "../src/skills";
import {
  buildAskUserQuestionForm,
  planStateMachineRefactor,
  runTasteAntiSlopAudit,
  runVibeToSpecPipeline,
} from "../src/pack_vibe";

describe("P6-02 Pack 1 -- Vibe Coders, PM, Scrum & UI/UX Wireframing", () => {
  test("SKILL.md loads cleanly via DDIPE, Vibe-to-Spec runs 3 Goldfish critics & /taste enforces a11y + 1440/360px", () => {
    const skill = loadSkillFromDir(path.resolve(import.meta.dir, "../skills/pack-vibe"));
    expect(skill.metadata.name).toBe("pack-vibe");

    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p602-"));
    try {
      const spec = runVibeToSpecPipeline(ws, "make a cool habit tracker", ["Daily check-in", "Streak chart"]);
      expect(fs.existsSync(spec.prdFile)).toBeTrue();
      expect(spec.critiques.map((c) => c.role)).toEqual(["Contrarian", "Edge-Case", "Token Economist"]);
      expect(spec.slices).toHaveLength(2);
      expect(spec.slices.every((s) => s.freshContext)).toBeTrue();

      const form = buildAskUserQuestionForm("auth", "Choose sign-in?", [
        { key: "A", label: "Local only", impact: "Zero backend" },
        { key: "B", label: "OAuth", impact: "Multi-device sync" },
      ]);
      expect(form.options).toHaveLength(2);

      const sm = planStateMachineRefactor(["isLoading", "isError"], ["idle", "loading", "ready", "error"]);
      expect(sm.axes).toEqual(["UX", "DX", "AX"]);

      const badTaste = runTasteAntiSlopAudit({
        htmlOrCss: `<img src="x.png"><style>body{background:linear-gradient(#7c3aed,#fff);outline:none;}</style>`,
        hasSmellMd: false,
        hasTypesetMd: false,
        desktop1440OverflowPx: 0,
        mobile360OverflowPx: 24,
      });
      expect(badTaste.ok).toBeFalse();
      expect(badTaste.issues.length).toBeGreaterThanOrEqual(5);

      const goodTaste = runTasteAntiSlopAudit({
        htmlOrCss: `<img src="x.png" alt="Logo"><style>button:focus-visible{outline:2px solid #111}</style>`,
        hasSmellMd: true,
        hasTypesetMd: true,
        desktop1440OverflowPx: 0,
        mobile360OverflowPx: 0,
      });
      expect(goodTaste.ok).toBeTrue();
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});