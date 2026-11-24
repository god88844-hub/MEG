import { describe, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { VerifierLoopLedger } from "../src/verifier_loop";
import { RiseHandshakeCoordinator } from "../src/handshake";
import { evaluateMirlGate } from "../src/checks_gate";
import { scanAntiStubAndFalseZero } from "../src/poteto_checks";

describe("Phase 4 Gate: Cheating-Builder Test Suite (100% Blocked)", () => {
  test("1. Empty diff is blocked by +DI handshake", () => {
    const hs = new RiseHandshakeCoordinator();
    const verdict = hs.evaluateDesignatedInvocation({
      designatedScript: "scripts/check.ps1",
      invokedCommands: ["scripts/check.ps1"],
      hasUnverifiedDiff: false,
      diffIsEmpty: true,
    });
    expect(verdict.allow).toBeFalse();
  });

  test("2. Edited test file is blocked by MiRL test_tampering gate", () => {
    const mirl = evaluateMirlGate({
      diffText: "- expect(res).toBe(42)\n+ expect(res).toBeDefined()",
      editedFiles: ["meg-plane/test/plane.test.ts"],
      hasProofArtifact: true,
      claimedDone: true,
    });
    expect(mirl.passed).toBeFalse();
    expect(mirl.violations).toContain("test_tampering");
  });

  test("3. Mocked pass is blocked by MiRL mocked_pass gate", () => {
    const mirl = evaluateMirlGate({
      diffText: "const verify = jest.fn(() => true); // mockReturnValue(true)",
      editedFiles: ["meg-plane/src/router.ts"],
      hasProofArtifact: true,
      claimedDone: true,
    });
    expect(mirl.passed).toBeFalse();
    expect(mirl.violations).toContain("mocked_pass");
  });

  test("4. False-zero catch is blocked by both Poteto scanner and MiRL gate", () => {
    const code = "try { return parseInvoice(raw); } catch (err) { return []; }";
    expect(scanAntiStubAndFalseZero(code).map((f) => f.kind)).toContain("false_zero_catch");
    const mirl = evaluateMirlGate({
      diffText: code,
      editedFiles: ["src/invoice.ts"],
      hasProofArtifact: true,
      claimedDone: true,
    });
    expect(mirl.passed).toBeFalse();
    expect(mirl.violations).toContain("silent_error_swallow");
  });

  test("5. Single-method OCR is blocked by +IR single-signature branch", () => {
    const hs = new RiseHandshakeCoordinator();
    const verdict = hs.evaluateIndependentReadings([{ method: "ocr_only", value: "100.00" }]);
    expect(verdict.allow).toBeFalse();
    expect(verdict.reason).toContain("single-signature");
  });

  test("6. Repeat loop is blocked by Loop 2 MAX_SAME=2 stall exit", () => {
    const ws = fs.mkdtempSync(path.join(os.tmpdir(), "meg-cheat-loop-"));
    try {
      const ledger = new VerifierLoopLedger(ws, "cheat-repeat");
      ledger.recordAttempt({
        rawError: "AssertionError at src/x.ts:10",
        hypothesis: "h1",
        diffHash: "d1",
        passedTests: 0,
        efc: { I: 0.3, V: 0.3, R: 0.5, M: 0.1 },
      });
      const second = ledger.recordAttempt({
        rawError: "AssertionError at src/x.ts:12",
        hypothesis: "h2",
        diffHash: "d2",
        passedTests: 0,
        efc: { I: 0.1, V: 0.1, R: 0.5, M: 0.0 },
      });
      expect(second.action).toBe("park_blocked");
      expect(second.pattern).toBe("Repetition");
    } finally {
      fs.rmSync(ws, { recursive: true, force: true });
    }
  });
});