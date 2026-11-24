import { describe, expect, test } from "bun:test";
import { RiseHandshakeCoordinator } from "../src/handshake";

describe("P4-02 Patched RISEHARNESS 4-Stage Handshake", () => {
  test("Unified +SE/+OO runs ONCE, records spec_check_failure_ledger (pytest --last-failed), fails open on infra error, and caps MAX_WITHHOLDS=1", async () => {
    const hs = new RiseHandshakeCoordinator();
    let runs = 0;

    // Failing test run -> withheld once + recorded in spec_check_failure_ledger
    const v1 = await hs.runUnifiedSeOo(
      { artifactSha: "sha-fail-1", verificationCmd: "pytest tests/", expectedExitCode: 0 },
      async () => {
        runs++;
        return { exitCode: 1, stdout: "", stderr: "1 failed" };
      },
    );
    expect(v1.allow).toBeFalse();
    expect(v1.withheld).toBeTrue();
    expect(v1.recommendedRerunCmd).toBe("pytest --last-failed");
    expect(hs.getFailureLedgerEntry("sha-fail-1")?.failedTestsCmd).toBe("pytest --last-failed");

    // Re-calling same SHA does NOT re-run sandbox (executed ONCE).
    const v1Repeat = await hs.runUnifiedSeOo(
      { artifactSha: "sha-fail-1", verificationCmd: "pytest tests/", expectedExitCode: 0 },
      async () => {
        runs++;
        return { exitCode: 1, stdout: "", stderr: "" };
      },
    );
    expect(runs).toBe(1);
    expect(v1Repeat.executedOnce).toBeFalse();

    // Sandbox infra failure fails OPEN.
    const infra = await hs.runUnifiedSeOo(
      { artifactSha: "sha-infra", verificationCmd: "pytest tests/", expectedExitCode: 0 },
      async () => ({ exitCode: -1, stdout: "", stderr: "", infraError: "docker daemon unreachable" }),
    );
    expect(infra.allow).toBeTrue();
    expect(infra.failOpenInfra).toBeTrue();
  });

  test("+DI blocks empty/unverified diffs and +IR catches both single-signature (single-method OCR) and conflicting readings", () => {
    const hs1 = new RiseHandshakeCoordinator();
    const diBlock = hs1.evaluateDesignatedInvocation({
      designatedScript: "scripts/check.ps1",
      invokedCommands: ["echo hi"],
      hasUnverifiedDiff: true,
      diffIsEmpty: true,
    });
    expect(diBlock.allow).toBeFalse();
    expect(diBlock.reason).toContain("empty diff");

    // Single-signature branch (+IR) blocks single-method OCR
    const irSingle = hs1.evaluateIndependentReadings([{ method: "tesseract_ocr", value: "42.0" }]);
    expect(irSingle.allow).toBeFalse();
    expect(irSingle.reason).toContain("single-signature");

    // Conflicting-readings branch (+IR) blocks disagreement across two independent methods
    const hs2 = new RiseHandshakeCoordinator();
    const irConflict = hs2.evaluateIndependentReadings([
      { method: "tesseract_ocr", value: "42.0" },
      { method: "pdf_text_layer", value: "49.5" },
    ]);
    expect(irConflict.allow).toBeFalse();
    expect(irConflict.reason).toContain("conflicting-readings");

    // Corroborated readings pass
    const irOk = hs2.evaluateIndependentReadings([
      { method: "tesseract_ocr", value: "42.0" },
      { method: "pdf_text_layer", value: "42.0" },
    ]);
    expect(irOk.allow).toBeTrue();
  });
});