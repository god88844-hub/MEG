import { describe, expect, test } from "bun:test";
import {
  buildMaintainerComprehensionQuiz,
  formatRemediationLint,
  runFastMutationGate,
  scanAntiStubAndFalseZero,
  verifyStatechartBfs,
} from "../src/poteto_checks";

describe("P4-05 Poteto Pyramid Linters, Mutation Gate & Statechart Checker", () => {
  test("detects TODO stubs, 501 handlers, false-zero catches & weakened assertions with [REMEDIATION FIX]", () => {
    const badCode = [
      "export function fetchItems() {",
      "  // TODO implement pagination",
      "  try { return load(); } catch (e) { return []; }",
      "}",
      "app.get('/x', (_req, res) => res.status(501).send('nope'));",
      "expect(true).toBe(true);",
    ].join("\n");

    const findings = scanAntiStubAndFalseZero(badCode);
    expect(findings.map((f) => f.kind)).toContain("todo_stub");
    expect(findings.map((f) => f.kind)).toContain("false_zero_catch");
    expect(findings.map((f) => f.kind)).toContain("http_501");
    expect(findings.map((f) => f.kind)).toContain("weakened_assertion");

    const formatted = formatRemediationLint("src/api.ts", findings);
    expect(formatted).toContain("[REMEDIATION FIX]:");
  });

  test("fast mutation gate rejects hollow tests and passes strong tests", async () => {
    const src = "export const add = (a: number, b: number) => a + b;";
    const mutants = [{ id: "m1", description: "+ to -", apply: (s: string) => s.replace("a + b", "a - b") }];

    const hollow = await runFastMutationGate(src, mutants, () => true);
    expect(hollow.passed).toBeFalse();
    expect(hollow.survivedIds).toEqual(["m1"]);

    const strong = await runFastMutationGate(src, mutants, (code) => code.includes("a + b"));
    expect(strong.passed).toBeTrue();
    expect(strong.killRate).toBe(1);
  });

  test("BFS Statechart verifier catches deadlocks & forbidden states, and Quiz triggers on >300-line diffs", () => {
    const broken = verifyStatechartBfs({
      initial: "INIT",
      terminalStates: ["DONE"],
      forbiddenStates: ["CORRUPT"],
      transitions: {
        INIT: ["RUN", "CORRUPT"],
        RUN: ["STUCK"],
        STUCK: [], // deadlock!
        CORRUPT: [],
      },
    });
    expect(broken.valid).toBeFalse();
    expect(broken.deadlockStates).toContain("STUCK");
    expect(broken.forbiddenReached).toContain("CORRUPT");

    const clean = verifyStatechartBfs({
      initial: "INIT",
      terminalStates: ["DONE"],
      forbiddenStates: ["CORRUPT"],
      transitions: { INIT: ["RUN"], RUN: ["DONE"], DONE: [] },
    });
    expect(clean.valid).toBeTrue();

    expect(buildMaintainerComprehensionQuiz(["src/a.ts"], 120).required).toBeFalse();
    const quiz = buildMaintainerComprehensionQuiz(["src/a.ts", "src/b.ts"], 350);
    expect(quiz.required).toBeTrue();
    expect(quiz.questions).toHaveLength(3);
  });
});