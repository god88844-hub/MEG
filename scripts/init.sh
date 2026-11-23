#!/usr/bin/env bash
set -euo pipefail

ALLOW_DIRTY=0
if [[ "${1:-}" == "--allow-dirty" ]]; then
  ALLOW_DIRTY=1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
FAILURES=0

echo "MEG Preflight Check (${REPO_ROOT})"

check_tool() {
  local name="$1"
  shift
  if ! command -v "$1" >/dev/null 2>&1; then
    printf "  [FAIL] %-8s not found on PATH\n" "${name}" >&2
    FAILURES=$((FAILURES + 1))
    return
  fi
  local out
  if out="$("$@" 2>&1)" && [[ -n "${out}" ]]; then
    printf "  [OK]   %-8s %s\n" "${name}" "${out}"
  else
    printf "  [FAIL] %-8s failed to run\n" "${name}" >&2
    FAILURES=$((FAILURES + 1))
  fi
}

check_tool "cargo" cargo --version
check_tool "bun"   bun --version
check_tool "git"   git --version

PY_BIN=""
if command -v python3 >/dev/null 2>&1; then
  PY_BIN="python3"
elif command -v python >/dev/null 2>&1; then
  PY_BIN="python"
fi

if [[ -z "${PY_BIN}" ]]; then
  printf "  [FAIL] %-8s not found on PATH\n" "python" >&2
  FAILURES=$((FAILURES + 1))
else
  PY_VER="$("${PY_BIN}" --version 2>&1 || true)"
  if "${PY_BIN}" -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 11) else 1)" 2>/dev/null; then
    printf "  [OK]   %-8s %s\n" "python" "${PY_VER}"
  else
    printf "  [FAIL] %-8s %s (requires Python 3.11+)\n" "python" "${PY_VER}" >&2
    FAILURES=$((FAILURES + 1))
  fi
fi

for rel in AGENTS.md docs/PLAN.md docs/STATE.md meg-rs/Cargo.toml meg-plane/package.json; do
  if [[ ! -f "${REPO_ROOT}/${rel}" ]]; then
    printf "  [FAIL] missing  %s\n" "${rel}" >&2
    FAILURES=$((FAILURES + 1))
  fi
done

DIRTY="$(git -C "${REPO_ROOT}" status --porcelain)"
if [[ -n "${DIRTY}" ]]; then
  COUNT="$(printf "%s\n" "${DIRTY}" | wc -l | tr -d ' ')"
  if [[ "${ALLOW_DIRTY}" -eq 1 ]]; then
    printf "  [WARN] tree     dirty (%s path(s); allowed via --allow-dirty)\n" "${COUNT}"
  else
    printf "  [FAIL] tree     working tree is not clean (%s path(s))\n" "${COUNT}" >&2
    printf "%s\n" "${DIRTY}" | head -n 10 >&2
    FAILURES=$((FAILURES + 1))
  fi
else
  echo "  [OK]   tree     clean working tree"
fi

if [[ "${FAILURES}" -gt 0 ]]; then
  echo "Preflight FAILED (${FAILURES} issue(s))." >&2
  exit 1
fi

echo "Preflight PASSED."