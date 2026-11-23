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
