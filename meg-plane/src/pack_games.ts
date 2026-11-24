// P6-04 Pack 3: 2D/3D Games, Physics & ARC-AGI-3 World Models (Domains 16-17).

import fs from "node:fs";
import path from "node:path";

export const M5_ECS_SEVEN_SUBSYSTEMS = [
  "Input",
  "Physics",
  "Collision",
  "AI",
  "Audio",
  "Render",
  "HUD",
] as const;

/** Scaffold M5 7-Subsystem ECS `SCHEMA.md` in `workspace`. */
export function scaffoldM5EcsSchema(workspace: string, gameTitle: string): string {
  const schemaFile = path.join(workspace, "SCHEMA.md");
  const lines = [
    `# ${gameTitle} -- M5 7-Subsystem ECS Schema`,
    ``,
    `## Subsystems (Fixed 60-FPS Step Order)`,
    ...M5_ECS_SEVEN_SUBSYSTEMS.map((sub, i) => `${i + 1}. **${sub}**`),
    ``,
    `## Hot-Loop Rules`,
    `- All per-frame tick code lives in \`*.hot.ts\` / \`*.hot.rs\`.`,
    `- Zero per-frame heap allocations (\`new Array\`, \`new Map\`, \`JSON.parse\`) and zero unseeded \`Math.random()\`.`,
    ``,
  ];
  fs.writeFileSync(schemaFile, lines.join("\n"), "utf8");
  return schemaFile;
}

/** `.hot.*` zero-alloc / zero-unseeded-RNG 60-FPS frame linter (TS twin of Rust Ring-0 linter). */
export function lintHotLoopTs(fileName: string, source: string): string[] {
  if (!fileName.includes(".hot.")) return [];
  const rules: [RegExp, string][] = [
    [/\bMath\.random\s*\(/, "unseeded Math.random() in .hot.*"],
    [/\bnew\s+(Array|Map|Set)\s*\(/, "per-frame heap allocation in .hot.*"],
    [/\bJSON\.parse\s*\(/, "per-frame JSON.parse() allocation in .hot.*"],
  ];
  const hits: string[] = [];
  const lines = source.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    for (const [re, reason] of rules) {
      if (re.test(line)) hits.push(`${fileName}:${i + 1}: ${reason}`);
    }
  }
  return hits;
}

/** WebGL 0-console-error verification gate. */
export function verifyWebGlZeroConsoleErrors(consoleMessages: readonly { level: string; text: string }[]): {
  ok: boolean;
  errors: string[];
} {
  const errors = consoleMessages
    .filter((m) => m.level === "error" || /webgl|shader|gl_invalid/i.test(m.text))
    .map((m) => m.text);
  return { ok: errors.length === 0, errors };
}

/** Automated BFS playtester finding shortest action path from start state to any winning state. */
export function runBfsPlaytester<S, A>(config: {
  initial: S;
  actions: readonly A[];
  keyOf: (state: S) => string;
  step: (state: S, action: A) => S;
  isWin: (state: S) => boolean;
  maxStates?: number;
}): { solvable: boolean; path: A[]; exploredStates: number } {
  const maxStates = config.maxStates ?? 10_000;
  const visited = new Set<string>([config.keyOf(config.initial)]);
  const queue: { state: S; path: A[] }[] = [{ state: config.initial, path: [] }];

  while (queue.length > 0 && visited.size <= maxStates) {
    const cur = queue.shift()!;
    if (config.isWin(cur.state)) {
      return { solvable: true, path: cur.path, exploredStates: visited.size };
    }
    for (const action of config.actions) {
      const next = config.step(cur.state, action);
      const key = config.keyOf(next);
      if (!visited.has(key)) {
        visited.add(key);
        queue.push({ state: next, path: [...cur.path, action] });
      }
    }
  }
  return { solvable: false, path: [], exploredStates: visited.size };
}

export interface WorldModelState {
  gridKey: string;
  score: number;
  status: "PLAYING" | "GAME_OVER" | "WIN";
}

export interface BacktestTransition {
  before: WorldModelState;
  action: string;
  expectedAfter: WorldModelState;
}

/**
 * Kepler 1.0 / Schema World-Model Runner:
 * - Auto-triggers `RESET` immediately when `status === "GAME_OVER"`.
 * - Certifies `step(state, action)` against recorded trajectories (requires 100% `run_backtest` accuracy).
 */
export function stepWithAutoReset(
  state: WorldModelState,
  action: string,
  modelStep: (state: WorldModelState, action: string) => WorldModelState,
  resetState: () => WorldModelState,
): { nextState: WorldModelState; autoResetApplied: boolean } {
  if (state.status === "GAME_OVER") {
    return { nextState: resetState(), autoResetApplied: true };
  }
  return { nextState: modelStep(state, action), autoResetApplied: false };
}

export function runWorldModelBacktest(
  transitions: readonly BacktestTransition[],
  modelStep: (state: WorldModelState, action: string) => WorldModelState,
): { certified: boolean; accuracy: number; mismatches: number } {
  if (transitions.length === 0) return { certified: false, accuracy: 0, mismatches: 0 };
  let matches = 0;
  for (const t of transitions) {
    const pred = modelStep(t.before, t.action);
    if (
      pred.gridKey === t.expectedAfter.gridKey &&
      pred.score === t.expectedAfter.score &&
      pred.status === t.expectedAfter.status
    ) {
      matches += 1;
    }
  }
  const accuracy = matches / transitions.length;
  return {
    certified: accuracy === 1.0,
    accuracy,
    mismatches: transitions.length - matches,
  };
}