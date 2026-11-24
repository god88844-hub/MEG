// P6-03 Pack 2: Web, Full-Stack, Backend, Mobile, Desktop, DB, Enterprise & QA (Domains 4-15).

export interface LspDiagnostic {
  file: string;
  line: number;
  severity: "error" | "warning";
  message: string;
}

export interface LspEditCheckResult {
  elapsedMs: number;
  within50msBudget: boolean;
  diagnostics: LspDiagnostic[];
  clean: boolean;
}

/** Sub-50ms LSP diagnostics on edit wrapper. */
export async function runLspDiagnosticsOnEdit(
  file: string,
  probe: (file: string) => Promise<LspDiagnostic[]> | LspDiagnostic[],
): Promise<LspEditCheckResult> {
  const t0 = performance.now();
  const diagnostics = await probe(file);
  const elapsedMs = Math.round((performance.now() - t0) * 100) / 100;
  return {
    elapsedMs,
    within50msBudget: elapsedMs <= 50,
    diagnostics,
    clean: !diagnostics.some((d) => d.severity === "error"),
  };
}

export type DapRuntime = "debugpy" | "dlv" | "lldb";

export function buildDapLaunchConfig(
  runtime: DapRuntime,
  program: string,
  breakpoints: readonly number[],
): { adapter: DapRuntime; command: string[]; program: string; breakpoints: number[] } {
  const command =
    runtime === "debugpy"
      ? ["python", "-m", "debugpy", "--listen", "127.0.0.1:5678", "--wait-for-client", program]
      : runtime === "dlv"
        ? ["dlv", "dap", "--listen=127.0.0.1:38697"]
        : ["lldb-vscode"];
  return { adapter: runtime, command, program, breakpoints: [...breakpoints] };
}

export interface BombadilFuzzStep {
  action: "click" | "type" | "navigate" | "back";
  target: string;
  consoleErrors: string[];
  uncaughtExceptions: string[];
}

/** Bombadil property-based UI fuzzing verifier: invariant holds iff 0 console errors & 0 uncaught exceptions across all steps. */
export function verifyBombadilUiFuzzTrace(steps: readonly BombadilFuzzStep[]): {
  passed: boolean;
  stepsRun: number;
  failures: string[];
} {
  const failures: string[] = [];
  for (let i = 0; i < steps.length; i++) {
    const s = steps[i]!;
    for (const err of s.consoleErrors) failures.push(`step ${i + 1} (${s.action} ${s.target}) consoleError: ${err}`);
    for (const exc of s.uncaughtExceptions) failures.push(`step ${i + 1} (${s.action} ${s.target}) uncaught: ${exc}`);
  }
  return { passed: failures.length === 0, stepsRun: steps.length, failures };
}

/** Deployment bootstrap-ordering check: ensures secrets/env -> db_migrate -> app_start -> healthcheck order. */
export function verifyBootstrapOrdering(stages: readonly string[]): { ok: boolean; errors: string[] } {
  const order = ["load_secrets", "db_migrate", "app_start", "healthcheck"];
  const errors: string[] = [];
  let prevIdx = -1;
  for (const required of order) {
    const idx = stages.indexOf(required);
    if (idx === -1) {
      errors.push(`Missing bootstrap stage '${required}'`);
    } else if (idx < prevIdx) {
      errors.push(`Bootstrap ordering violation: '${required}' appears before prerequisite stage`);
    } else {
      prevIdx = idx;
    }
  }
  return { ok: errors.length === 0, errors };
}

export interface AccessibilityNode {
  role: string;
  name: string;
  value?: string;
  children?: AccessibilityNode[];
}

const PII_PATTERNS: readonly [RegExp, string][] = [
  [/\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g, "<PII_EMAIL>"],
  [/\b\d{3}-\d{2}-\d{4}\b/g, "<PII_SSN>"],
  [/\b(?:\d[ -]*?){13,16}\b/g, "<PII_CARD>"],
  [/\b\+?91[- ]?\d{10}\b|\b\d{10}\b/g, "<PII_PHONE>"],
];

function scrubPiiText(text: string): string {
  let out = text;
  for (const [re, token] of PII_PATTERNS) {
    out = out.replace(re, token);
  }
  return out;
}

/** Desktop GUI OS Accessibility Tree bridge (`agent-desktop` JSON tree with recursive PII scrubbing). */
export function scrubAccessibilityTreePii(node: AccessibilityNode): AccessibilityNode {
  return {
    role: node.role,
    name: scrubPiiText(node.name),
    value: node.value !== undefined ? scrubPiiText(node.value) : undefined,
    children: node.children?.map(scrubAccessibilityTreePii),
  };
}

export interface PercentCellAudit {
  cell: string;
  rawValue: number;
  numberFormat: string;
  normalizedPercent: number;
  convertedFromFraction: boolean;
}

/**
 * Binary Office/Data skill (`s7->s30` `PercentCheckHarness` for `.xlsx`):
 * Detects decimal ratios in `(0, 1)` formatted as plain numbers where a percentage (`*100`)
 * was expected, and normalizes them deterministically.
 */
export function runPercentCheckHarness(
  cells: readonly { cell: string; value: number; numberFormat: string; expectPercent: boolean }[],
): { ok: boolean; audits: PercentCellAudit[]; warnings: string[] } {
  const audits: PercentCellAudit[] = [];
  const warnings: string[] = [];

  for (const c of cells) {
    const hasPctFmt = c.numberFormat.includes("%");
    const isFraction = c.value > 0 && c.value < 1;
    let normalizedPercent = c.value;
    let convertedFromFraction = false;

    if (c.expectPercent && isFraction && !hasPctFmt) {
      normalizedPercent = Math.round(c.value * 100 * 10_000) / 10_000;
      convertedFromFraction = true;
      warnings.push(
        `PercentCheckHarness [${c.cell}]: fraction ${c.value} lacked '%' format -> normalized to ${normalizedPercent}%`,
      );
    } else if (c.expectPercent && hasPctFmt && isFraction) {
      normalizedPercent = Math.round(c.value * 100 * 10_000) / 10_000;
    }

    audits.push({
      cell: c.cell,
      rawValue: c.value,
      numberFormat: c.numberFormat,
      normalizedPercent,
      convertedFromFraction,
    });
  }

  return { ok: warnings.length === 0, audits, warnings };
}