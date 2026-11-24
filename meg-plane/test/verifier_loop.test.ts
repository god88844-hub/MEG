import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  VerifierLoopLedger,
  buildAdaptiveReplayBrief,
  computeEfcScore,
  detectLoopPattern,
  evaluateDeclineCraft,
  fingerprintError,
  formatStructuredFeedback,
  routeDiagnosticLayer,
  runCheckCannotFailGate,
} from "../src/verifier_loop";

describe("P4-01 Verified Work Loop (Loop 2) & Stall/EFC Guards", () => {
  test("check_cannot_fail proves Green -> Red -> Green and blocks hollow checks", async () => {
    let broken = false;
    const good = await runCheckCannotFailGate({
      runCheck: () => !broken,
      plantFault: () => { broken = true; },
      revertFault: () => { broken = false; },
    });
    expect(good.valid).toBeTrue();

    const hollow = await runCheckCannotFailGate({
      runCheck: () => true, // never fails even with fault!
      plantFault: () => {},
      revertFault: () => {},
    });
    expect(hollow.valid).toBeFalse();
    expect(hollow.reason).toContain("CHECK_CANNOT_FAIL");
  });

  test("masks paths/digits in phi(e), detects Repetition/Stagnation/A->B->A Cycling, parks at MAX_SAME=2, and triggers Karpathy Clean-Slate Replay after >4 retries", () => {
    const f1 = fingerprintError("Error at C:\\repo\\a.ts:42 expected 123");
    const f2 = fingerprintError("Error at /tmp/other/b.ts:99 expected 456");
    expect(f1).toBe(f2);

    const card = formatStructuredFeedback({
      failed: "test_math",
      expected: "4",
      saw: "5",
      location: "src/math.ts:12",
    });
    expect(card.length).toBeLessThanOrEqual(3000);
    expect(card).toContain("FAILED:");
    expect(card).toContain("EXPECTED:");
    expect(card).toContain("SAW:");

    expect(
      detectLoopPattern([
        { attempt: 1, fingerprint: "A", hypothesis: "h1", diffHash: "d1", passedTests: 1, timestamp: 1 },
        { attempt: 2, fingerprint: "B", hypothesis: "h2", diffHash: "d2", passedTests: 1, timestamp: 2 },
        { attempt: 3, fingerprint: "A", hypothesis: "h3", diffHash: "d3", passedTests: 1, timestamp: 3 },
      ]),
    ).toBe("A->B->A Cycling");

    expect(computeEfcScore({ I: 0.8, V: 0.5, R: 1.0, M: 0.5 })).toBeCloseTo(0.2);

    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p401-"));
    try {
      const ledger = new VerifierLoopLedger(ws, "task-loop");
      const d1 = ledger.recordAttempt({
        rawError: "TypeError at /a.ts:10",
        hypothesis: "try cast",
        diffHash: "h1",
        passedTests: 1,
        efc: { I: 0.8, V: 0.8, R: 0.8, M: 0.8 },
      });
      expect(d1.action).toBe("continue");

      // Second identical fingerprint triggers MAX_SAME=2 -> park on blocked/<id>.
      const d2 = ledger.recordAttempt({
        rawError: "TypeError at /b.ts:99",
        hypothesis: "try cast again",
        diffHash: "h2",
        passedTests: 1,
        efc: { I: 0.2, V: 0.2, R: 0.5, M: 0.2 },
      });
      expect(d2.action).toBe("park_blocked");
      expect(d2.parkedBranch).toBe("blocked/task-loop");
      expect(fs.existsSync(path.join(ws, ".loop", "state.json"))).toBeTrue();

      // Fresh task hitting >4 messy retries triggers Karpathy Clean-Slate Replay.
      const ws2 = fs.mkdtempSync(path.join(os.tmpdir(), "meg-p401-cs-"));
      try {
        const ledger2 = new VerifierLoopLedger(ws2, "task-cs");
        let lastAction = "";
        for (let i = 1; i <= 5; i++) {
          const res = ledger2.recordAttempt({
            rawError: `DistinctFailureKind_${"abcde"[i - 1]}`,
            hypothesis: `hyp-${i}`,
            diffHash: `dh-${i}`,
            passedTests: i,
            efc: { I: 0.7, V: 0.7, R: 0.7, M: 0.7 },
          });
          lastAction = res.action;
        }
        expect(lastAction).toBe("clean_slate_replay");
        expect(buildAdaptiveReplayBrief(ledger2.snapshot().tried)).toContain("Clean-Slate Pass");
      } finally {
        fs.rmSync(ws2, { recursive: true, force: true });
      }
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });

  test("Concentric 4-Layer Diagnostic Router and DeclineCraft calibrated abstention work cleanly", () => {
    expect(routeDiagnosticLayer({ sandboxOrToolInfraFailed: true, loopPattern: "Repetition" })).toBe("Harness");
    expect(routeDiagnosticLayer({ loopPattern: "Stagnation" })).toBe("Loop");
    expect(routeDiagnosticLayer({ missingOrStaleContext: true })).toBe("Context");
    expect(routeDiagnosticLayer({ ambiguousInstruction: true })).toBe("Prompt");

    expect(
      evaluateDeclineCraft({ evidenceCoverage: 0.4, contradictorySignals: true, missingRequiredPermissions: false })
        .abstain,
    ).toBeTrue();
    expect(
      evaluateDeclineCraft({ evidenceCoverage: 0.9, contradictorySignals: false, missingRequiredPermissions: false })
        .abstain,
    ).toBeFalse();
  });
});