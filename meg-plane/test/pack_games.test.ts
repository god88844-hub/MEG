import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { loadSkillFromDir } from "../src/skills";
import {
  M5_ECS_SEVEN_SUBSYSTEMS,
  lintHotLoopTs,
  runBfsPlaytester,
  runWorldModelBacktest,
  scaffoldM5EcsSchema,
  stepWithAutoReset,
  verifyWebGlZeroConsoleErrors,
} from "../src/pack_games";

describe("P6-04 Pack 3 -- 2D/3D Games, Physics & ARC-AGI-3 World Models", () => {
  test("SKILL.md loads cleanly, scaffolds M5 7-Subsystem ECS, enforces .hot.* lint, WebGL 0-error, BFS playtester & Kepler 100% backtest + auto-RESET", () => {
    const skill = loadSkillFromDir(path.resolve(import.meta.dir, "../skills/pack-games"));
    expect(skill.metadata.name).toBe("pack-games");

    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p604-"));
    try {
      const schema = scaffoldM5EcsSchema(ws, "OrbitDash");
      const raw = fs.readFileSync(schema, "utf8");
      expect(M5_ECS_SEVEN_SUBSYSTEMS).toHaveLength(7);
      for (const sub of M5_ECS_SEVEN_SUBSYSTEMS) expect(raw).toContain(sub);

      expect(lintHotLoopTs("physics.hot.ts", "const x = Math.random(); new Array(4);")).toHaveLength(2);
      expect(lintHotLoopTs("boot.ts", "Math.random()")).toHaveLength(0);

      expect(verifyWebGlZeroConsoleErrors([{ level: "info", text: "fps=60" }]).ok).toBeTrue();
      expect(verifyWebGlZeroConsoleErrors([{ level: "error", text: "WebGL shader compile failed" }]).ok).toBeFalse();

      const bfs = runBfsPlaytester({
        initial: 0,
        actions: ["+1", "+2"] as const,
        keyOf: (s) => String(s),
        step: (s, a) => (a === "+1" ? s + 1 : s + 2),
        isWin: (s) => s === 5,
      });
      expect(bfs.solvable).toBeTrue();
      expect(bfs.path).toHaveLength(3);

      const modelStep = (st: { gridKey: string; score: number; status: "PLAYING" | "GAME_OVER" | "WIN" }, act: string) =>
        act === "GOAL"
          ? { gridKey: "win", score: st.score + 10, status: "WIN" as const }
          : { gridKey: "dead", score: st.score, status: "GAME_OVER" as const };

      const resetRes = stepWithAutoReset(
        { gridKey: "dead", score: 0, status: "GAME_OVER" },
        "LEFT",
        modelStep,
        () => ({ gridKey: "start", score: 0, status: "PLAYING" }),
      );
      expect(resetRes.autoResetApplied).toBeTrue();
      expect(resetRes.nextState.status).toBe("PLAYING");

      const bt = runWorldModelBacktest(
        [
          {
            before: { gridKey: "start", score: 0, status: "PLAYING" },
            action: "GOAL",
            expectedAfter: { gridKey: "win", score: 10, status: "WIN" },
          },
        ],
        modelStep,
      );
      expect(bt.certified).toBeTrue();
      expect(bt.accuracy).toBe(1);
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});