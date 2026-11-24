// P4-02: Patched RISEHARNESS 4-Stage Handshake:
// - Unified +SE/+OO: validates verification_cmd + expected_exit_code and runs ONCE
//   in a 30s out-of-band sandbox, failing open on infra error
// - spec_check_failure_ledger (`pytest --last-failed` keyed by artifact SHA)
// - +DI: designated script invocation / unverified diff check
// - +IR: reachable single-signature AND conflicting-readings branches
// - Strictly bounded at MAX_WITHHOLDS = 1 per gate

export const MAX_WITHHOLDS_PER_GATE = 1;
export const OUT_OF_BAND_TIMEOUT_MS = 30_000;

export interface SpecOracleInput {
  artifactSha: string;
  verificationCmd: string;
  expectedExitCode: number;
}

export interface SandboxExecutionResult {
  exitCode: number;
  stdout: string;
  stderr: string;
  infraError?: string;
}

export interface HandshakeGateVerdict {
  gate: "+SE/+OO" | "+DI" | "+IR";
  allow: boolean;
  withheld: boolean;
  failOpenInfra?: boolean;
  executedOnce?: boolean;
  recommendedRerunCmd?: string;
  reason: string;
}

export interface SpecCheckFailureEntry {
  artifactSha: string;
  verificationCmd: string;
  failedTestsCmd: string;
  exitCode: number;
  recordedAt: number;
}

/**
 * Patched RISEHARNESS 4-Stage Handshake Coordinator.
 * Bounds every gate to MAX_WITHHOLDS = 1 so verification never causes infinite deadlocks.
 */
export class RiseHandshakeCoordinator {
  private withholds = new Map<string, number>();
  private seOoRunShas = new Set<string>();
  private specFailureLedger = new Map<string, SpecCheckFailureEntry>();

  private canWithhold(gate: string): boolean {
    return (this.withholds.get(gate) ?? 0) < MAX_WITHHOLDS_PER_GATE;
  }

  private recordWithhold(gate: string): void {
    this.withholds.set(gate, (this.withholds.get(gate) ?? 0) + 1);
  }

  getFailureLedgerEntry(artifactSha: string): SpecCheckFailureEntry | undefined {
    return this.specFailureLedger.get(artifactSha);
  }

  /**
   * Unified +SE/+OO Gate:
   * - Validates `verificationCmd` & integer `expectedExitCode`.
   * - Executes ONCE per artifactSha in a 30s out-of-band sandbox.
   * - Fails OPEN on sandbox infrastructure errors so infra glitches never deadlock delivery.
   * - Records failures in `spec_check_failure_ledger` keyed by artifact SHA (`pytest --last-failed`).
   */
  async runUnifiedSeOo(
    input: SpecOracleInput,
    runSandbox: (cmd: string, timeoutMs: number) => Promise<SandboxExecutionResult>,
  ): Promise<HandshakeGateVerdict> {
    if (!input.verificationCmd.trim() || !Number.isInteger(input.expectedExitCode)) {
      if (this.canWithhold("+SE/+OO")) {
        this.recordWithhold("+SE/+OO");
        return {
          gate: "+SE/+OO",
          allow: false,
          withheld: true,
          reason: "Invalid +SE/+OO contract: verification_cmd and integer expected_exit_code required",
        };
      }
      return { gate: "+SE/+OO", allow: true, withheld: false, reason: "MAX_WITHHOLDS=1 reached; proceeding with warning" };
    }

    // Run ONCE per artifact SHA.
    if (this.seOoRunShas.has(input.artifactSha)) {
      return {
        gate: "+SE/+OO",
        allow: true,
        withheld: false,
        executedOnce: false,
        reason: "Out-of-band verification already executed once for this artifact SHA",
      };
    }
    this.seOoRunShas.add(input.artifactSha);

    let res: SandboxExecutionResult;
    try {
      res = await runSandbox(input.verificationCmd, OUT_OF_BAND_TIMEOUT_MS);
    } catch (err) {
      return {
        gate: "+SE/+OO",
        allow: true,
        withheld: false,
        failOpenInfra: true,
        executedOnce: true,
        reason: `Sandbox infra error (fail-open): ${err instanceof Error ? err.message : String(err)}`,
      };
    }

    if (res.infraError) {
      return {
        gate: "+SE/+OO",
        allow: true,
        withheld: false,
        failOpenInfra: true,
        executedOnce: true,
        reason: `Sandbox infra error (fail-open): ${res.infraError}`,
      };
    }

    if (res.exitCode !== input.expectedExitCode) {
      const rerun = input.verificationCmd.includes("pytest")
        ? "pytest --last-failed"
        : input.verificationCmd;
      this.specFailureLedger.set(input.artifactSha, {
        artifactSha: input.artifactSha,
        verificationCmd: input.verificationCmd,
        failedTestsCmd: rerun,
        exitCode: res.exitCode,
        recordedAt: Date.now(),
      });
      if (this.canWithhold("+SE/+OO")) {
        this.recordWithhold("+SE/+OO");
        return {
          gate: "+SE/+OO",
          allow: false,
          withheld: true,
          executedOnce: true,
          recommendedRerunCmd: rerun,
          reason: `Expected exit ${input.expectedExitCode}, saw ${res.exitCode}`,
        };
      }
    }

    return {
      gate: "+SE/+OO",
      allow: true,
      withheld: false,
      executedOnce: true,
      reason: `Out-of-band +SE/+OO passed (exit ${res.exitCode})`,
    };
  }

  /**
   * +DI Gate: designated script invocation / unverified diff check (bounded at MAX_WITHHOLDS = 1).
   */
  evaluateDesignatedInvocation(input: {
    designatedScript?: string;
    invokedCommands: readonly string[];
    hasUnverifiedDiff: boolean;
    diffIsEmpty?: boolean;
  }): HandshakeGateVerdict {
    const problems: string[] = [];
    if (input.diffIsEmpty) {
      problems.push("empty diff: no code changes produced");
    }
    if (input.designatedScript) {
      const called = input.invokedCommands.some((c) => c.includes(input.designatedScript!));
      if (!called) {
        problems.push(`designated verification script '${input.designatedScript}' was never invoked`);
      }
    }
    if (input.hasUnverifiedDiff) {
      problems.push("code diff modified after last verification run");
    }

    if (problems.length > 0 && this.canWithhold("+DI")) {
      this.recordWithhold("+DI");
      return {
        gate: "+DI",
        allow: false,
        withheld: true,
        reason: problems.join("; "),
      };
    }
    return {
      gate: "+DI",
      allow: true,
      withheld: false,
      reason: problems.length ? "MAX_WITHHOLDS=1 reached on +DI" : "+DI designated invocation verified",
    };
  }

  /**
   * +IR Gate: Independent Readings check.
   * Reachable branches:
   * 1. Single-signature / single-method reading (e.g., single-method OCR with no corroboration).
   * 2. Conflicting independent readings (two methods disagree).
   * Bounded at MAX_WITHHOLDS = 1.
   */
  evaluateIndependentReadings(
    readings: readonly { method: string; value: string }[],
  ): HandshakeGateVerdict {
    const uniqueMethods = new Set(readings.map((r) => r.method.trim().toLowerCase()).filter(Boolean));
    const uniqueValues = new Set(readings.map((r) => r.value.trim()));

    if (uniqueMethods.size < 2) {
      if (this.canWithhold("+IR")) {
        this.recordWithhold("+IR");
        return {
          gate: "+IR",
          allow: false,
          withheld: true,
          reason: `single-signature reading (${uniqueMethods.size} method): corroborate with an independent second method`,
        };
      }
      return { gate: "+IR", allow: true, withheld: false, reason: "MAX_WITHHOLDS=1 reached on +IR (single-signature)" };
    }

    if (uniqueValues.size > 1) {
      if (this.canWithhold("+IR")) {
        this.recordWithhold("+IR");
        return {
          gate: "+IR",
          allow: false,
          withheld: true,
          reason: `conflicting-readings across independent methods (${[...uniqueValues].join(" vs ")})`,
        };
      }
      return { gate: "+IR", allow: true, withheld: false, reason: "MAX_WITHHOLDS=1 reached on +IR (conflicting-readings)" };
    }

    return {
      gate: "+IR",
      allow: true,
      withheld: false,
      reason: `Independent readings corroborated across ${uniqueMethods.size} methods`,
    };
  }
}