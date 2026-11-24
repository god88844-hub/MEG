import { describe, expect, test } from "bun:test";
import path from "node:path";
import { loadSkillFromDir } from "../src/skills";
import { CONTEXT_LIMITS, compileContext } from "../src/context";

describe("Phase 6 Gate (Tier 2): All 6 Lazy Domain Packs Clean & Base Prompt Strictly 6 Core Tools", () => {
  test("all 6 domain pack SKILL.md files pass DDIPE scan and base prompt stays capped at 6 tools", () => {
    const packs = [
      "pack-vibe",
      "pack-appdev",
      "pack-games",
      "pack-visual-motion",
      "pack-systems",
      "pack-daily",
    ];
    for (const pack of packs) {
      const loaded = loadSkillFromDir(path.resolve(import.meta.dir, `../skills/${pack}`), { thirdParty: true });
      expect(loaded.metadata.name).toBe(pack);
    }

    expect(CONTEXT_LIMITS.MAX_TOOLS).toBe(6);
    const compiled = compileContext({
      rules: ["Stay verified"],
      tools: [
        { name: "bash", schema: "{}" },
        { name: "read_file", schema: "{}" },
        { name: "write_file", schema: "{}" },
        { name: "edit_file", schema: "{}" },
        { name: "glob_grep", schema: "{}" },
        { name: "finish", schema: "{}" },
        { name: "extra_domain_tool_must_be_cut", schema: "{}" },
      ],
      examples: [],
      goal: "Verify 6-tool base cap",
      turnNumber: 1,
    });
    expect(compiled.cutReport.toolsKept).toBe(6);
    expect(compiled.staticPrefix).not.toContain("extra_domain_tool_must_be_cut");
  });
});