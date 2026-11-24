// Direct Security Task Router (AHE 4.4): subagent delegation strips benchmark
// framing and caused 0/14 solves on security/crypto tasks; root-thread
// execution solved 6/6. These tasks never delegate. Also enforces the 3–5
// subagent hand-off cap on everything else.

const SECURITY_TASKS: ReadonlySet<string> = new Set([
  "filter-js-from-html",
  "break-filter-js-from-html",
  "password-recovery",
  "crack-7z-hash",
  "feal-linear-cryptanalysis",
  "feal-differential-cryptanalysis",
  "vulnerable-secret",
  "sanitize-git-repo",
  "git-leak-recovery",
  "fix-code-vulnerability",
  "openssl-selfsigned-cert",
]);

const SECURITY_PROMPT_RE =
  /\b(xss|cross-site|exploit|payload|injection|crack|decrypt|cryptanalysis|secret key|red-team|7z hash|recover the password|password.recovery)\b/i;

export function isSecurityOrCryptoTask(taskName: string | null, prompt: string): boolean {
  if (taskName && SECURITY_TASKS.has(taskName)) return true;
  return SECURITY_PROMPT_RE.test(prompt ?? "");
}

export type RouteVerdict =
  | { kind: "root-thread"; reason: string }
  | { kind: "delegate"; reason: string };

export function routeTask(
  taskName: string | null,
  prompt: string,
  handOffsSoFar: number,
): RouteVerdict {
  if (isSecurityOrCryptoTask(taskName, prompt)) {
    return {
      kind: "root-thread",
      reason: "security/crypto task: delegation strips benchmark framing (0/14 delegated vs 6/6 root) — execute in the root thread",
    };
  }
  if (handOffsSoFar >= 5) {
    return { kind: "root-thread", reason: "hand-off cap (5) reached — finish in the root thread" };
  }
  return { kind: "delegate", reason: `hand-off ${handOffsSoFar + 1} of max 5` };
}

// ===========================================================================
// P3-05: DDIPE Skill Supply-Chain Scanner
// Scans third-party SKILL.md files and embedded code blocks for implicit
// payload execution before loading.
// ===========================================================================

export interface DdipeFinding {
  rule: string;
  severity: "critical" | "high";
  line: number;
  snippet: string;
  inCodeBlock: boolean;
}

export interface DdipeScanReport {
  safe: boolean;
  source: string;
  findings: DdipeFinding[];
}

const DDIPE_RULES: readonly { rule: string; severity: "critical" | "high"; pattern: RegExp }[] = [
  { rule: "pip_conf_hijack", severity: "critical", pattern: /\bpip\.conf\b|\bpip\.ini\b/i },
  { rule: "pip_extra_index_url", severity: "critical", pattern: /--extra-index-url|\bextra-index-url\b|\bPIP_EXTRA_INDEX_URL\b/i },
  { rule: "python_requests_exfil", severity: "critical", pattern: /\brequests\.(post|put)\s*\(/i },
  { rule: "curl_data_exfil", severity: "critical", pattern: /\bcurl\b[^\n]*(\s-d\s|\s--data\b|\s--data-binary\b|\s-F\s)/i },
  { rule: "ssh_authorized_keys", severity: "critical", pattern: /\bauthorized_keys\b/i },
  { rule: "ssh_dir_access", severity: "critical", pattern: /(^|[\s"'`~/$])\.ssh(\/|\\|\b)/i },
  { rule: "pipe_to_shell", severity: "high", pattern: /\b(curl|wget)\b[^\n]*\|\s*(sh|bash|zsh|pwsh|powershell)\b/i },
  { rule: "base64_shell_exec", severity: "high", pattern: /base64\s+(-d|--decode)\s*\|\s*(sh|bash|python|node)\b/i },
];

/**
 * Scan raw `SKILL.md` content (both prose and fenced code blocks) for DDIPE
 * supply-chain attack vectors. Any finding blocks loading (`safe: false`).
 */
export function scanSkillSupplyChain(content: string, source = "SKILL.md"): DdipeScanReport {
  const lines = content.split(/\r?\n/);
  const findings: DdipeFinding[] = [];
  let inCodeBlock = false;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    if (/^\s*```/.test(line)) {
      inCodeBlock = !inCodeBlock;
      continue;
    }
    for (const spec of DDIPE_RULES) {
      if (spec.pattern.test(line)) {
        findings.push({
          rule: spec.rule,
          severity: spec.severity,
          line: i + 1,
          snippet: line.trim().slice(0, 140),
          inCodeBlock,
        });
      }
    }
  }

  return {
    safe: findings.length === 0,
    source,
    findings,
  };
}