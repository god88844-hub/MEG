// P4-05: Poteto Pyramid Linters, Fast Mutation Gate, Statechart/TLA+ BFS Verifier
// & Optional 3-Question Maintainer Comprehension Quiz (>300-line diffs).

export interface StubFinding {
  kind: "todo_stub" | "http_501" | "false_zero_catch" | "weakened_assertion";
  line: number;
  snippet: string;
  remediation: string;
}

const ANTI_STUB_RULES: readonly {
  kind: StubFinding["kind"];
  re: RegExp;
  remediation: string;
}[] = [
  {
    kind: "todo_stub",
    re: /\b(TODO|FIXME|unimplemented!|todo!)\b|throw new Error\(["']not implemented/i,
    remediation: "Implement the concrete behavior or remove dead placeholder code before verification.",
  },
  {
    kind: "http_501",
    re: /\b(501|NOT_IMPLEMENTED)\b|status\(\s*501\s*\)/,
    remediation: "Replace HTTP 501 stub handler with a working implementation and real integration test.",
  },
  {
    kind: "false_zero_catch",
    re: /catch\s*(\([^)]*\))?\s*\{\s*(return\s*(\[\]|\{\}|0|false|null|undefined)?;?)?\s*\}/,
    remediation: "Do not swallow exceptions into empty/zero fallbacks; propagate or handle errors explicitly.",
  },
  {
    kind: "weakened_assertion",
    re: /(expect\(\s*true\s*\)\.(toBe|toEqual)\(\s*true\s*\)|assert!\(\s*true\s*\)|\.skip\(|test\.skip\b|xit\()/,
    remediation: "Restore strong behavioral assertions; never skip tests or assert tautologies.",
  },
];

/** AST/pattern anti-stub & false-zero scanner for candidate source files. */
export function scanAntiStubAndFalseZero(source: string): StubFinding[] {
  const lines = source.split(/\r?\n/);
  const findings: StubFinding[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    for (const rule of ANTI_STUB_RULES) {
      if (rule.re.test(line)) {
        findings.push({
          kind: rule.kind,
          line: i + 1,
          snippet: line.trim().slice(0, 140),
          remediation: rule.remediation,
        });
      }
    }
  }
  return findings;
}

/** Remediation-Injecting Linter formatter (`[REMEDIATION FIX]: ...`). */
export function formatRemediationLint(filePath: string, findings: readonly StubFinding[]): string {
  if (findings.length === 0) return "";
  return findings
    .map(
      (f) =>
        `${filePath}:${f.line} [${f.kind}] ${f.snippet}\n  [REMEDIATION FIX]: ${f.remediation}`,
    )
    .join("\n");
}

export interface MutantSpec {
  id: string;
  description: string;
  apply: (source: string) => string;
}

export interface MutationGateResult {
  passed: boolean;
  totalMutants: number;
  killedMutants: number;
  survivedIds: string[];
  killRate: number;
}

/**
 * tesearch-style fast mutation check:
 * Applies lightweight semantic mutants to `source`; if tests still pass on a mutant
 * (survived), the test suite is hollow and rejected.
 */
export async function runFastMutationGate(
  source: string,
  mutants: readonly MutantSpec[],
  runTestSuite: (mutatedSource: string) => Promise<boolean> | boolean,
  minKillRate = 1.0,
): Promise<MutationGateResult> {
  const survivedIds: string[] = [];
  let killedMutants = 0;
  let evaluated = 0;

  for (const m of mutants) {
    const mutated = m.apply(source);
    if (mutated === source) continue;
    evaluated += 1;
    const testsStillPassed = await runTestSuite(mutated);
    if (testsStillPassed) {
      survivedIds.push(m.id);
    } else {
      killedMutants += 1;
    }
  }

  const killRate = evaluated === 0 ? 1.0 : killedMutants / evaluated;
  return {
    passed: evaluated > 0 && killRate >= minKillRate,
    totalMutants: evaluated,
    killedMutants,
    survivedIds,
    killRate,
  };
}

export interface StatechartSpec {
  initial: string;
  terminalStates: readonly string[];
  forbiddenStates: readonly string[];
  transitions: Record<string, readonly string[]>;
}

export interface StatechartVerificationResult {
  valid: boolean;
  reachableStates: string[];
  deadlockStates: string[];
  forbiddenReached: string[];
}

/**
 * BFS Statechart / TLA+ reachability verifier:
 * Detects deadlocks (non-terminal states with 0 outgoing transitions) and reachable forbidden states.
 */
export function verifyStatechartBfs(spec: StatechartSpec): StatechartVerificationResult {
  const terminal = new Set(spec.terminalStates);
  const forbidden = new Set(spec.forbiddenStates);
  const visited = new Set<string>();
  const queue: string[] = [spec.initial];
  const deadlockStates: string[] = [];
  const forbiddenReached: string[] = [];

  while (queue.length > 0) {
    const state = queue.shift()!;
    if (visited.has(state)) continue;
    visited.add(state);

    if (forbidden.has(state)) {
      forbiddenReached.push(state);
    }
    const nexts = spec.transitions[state] ?? [];
    if (nexts.length === 0 && !terminal.has(state)) {
      deadlockStates.push(state);
    }
    for (const nxt of nexts) {
      if (!visited.has(nxt)) queue.push(nxt);
    }
  }

  return {
    valid: deadlockStates.length === 0 && forbiddenReached.length === 0,
    reachableStates: [...visited],
    deadlockStates,
    forbiddenReached,
  };
}

export const QUIZ_DIFF_LINE_THRESHOLD = 300;

export interface MaintainerQuiz {
  required: boolean;
  changedLines: number;
  questions: string[];
}

/** Optional 3-question Maintainer Comprehension Quiz triggered on diffs >300 lines. */
export function buildMaintainerComprehensionQuiz(
  changedFiles: readonly string[],
  changedLines: number,
): MaintainerQuiz {
  if (changedLines <= QUIZ_DIFF_LINE_THRESHOLD) {
    return { required: false, changedLines, questions: [] };
  }
  const primary = changedFiles[0] ?? "the modified module";
  const secondary = changedFiles[1] ?? "downstream callers";
  return {
    required: true,
    changedLines,
    questions: [
      `1. What invariant in '${primary}' breaks if the main conditional is inverted?`,
      `2. How does this change avoid regressions or state drift in '${secondary}'?`,
      `3. Which failure mode is rolled back (and how) if verification fails halfway through?`,
    ],
  };
}