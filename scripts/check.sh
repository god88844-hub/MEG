#!/usr/bin/env bash
set -euo pipefail

FAST=0
SKIP_RELEASE=0
CLIPPY=0
for arg in "$@"; do
  case "$arg" in
    --fast) FAST=1 ;;
    --skip-release) SKIP_RELEASE=1 ;;
    --clippy) CLIPPY=1 ;;
    *) echo "Unknown flag: $arg" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MEG_RS="${REPO_ROOT}/meg-rs"
MEG_PLANE="${REPO_ROOT}/meg-plane"
MEG_EVAL="${REPO_ROOT}/meg-eval"

run_step() {
  local label="$1"
  local dir="$2"
  shift 2
  echo "==> ${label}"
  (cd "${dir}" && "$@")
  echo "[PASS] ${label}"
}

if [[ "${FAST}" -eq 1 ]]; then
  run_step "Rust fast check (meg-core lib)" "${MEG_RS}" cargo test --locked -p meg-core --lib truncate::tests::short_text_untouched
  run_step "Bun fast check (plane.test.ts)" "${MEG_PLANE}" bun test test/plane.test.ts
  echo "Fast check PASSED."
  exit 0
fi

run_step "Rust workspace tests (--locked)" "${MEG_RS}" cargo test --locked --workspace
if [[ "${CLIPPY}" -eq 1 ]]; then
  run_step "Rust clippy (--locked --workspace)" "${MEG_RS}" cargo clippy --locked --workspace --all-targets
fi
if [[ "${SKIP_RELEASE}" -eq 0 ]]; then
  run_step "Rust release build (--locked)" "${MEG_RS}" cargo build --locked --release --workspace
fi
run_step "Bun control-plane tests" "${MEG_PLANE}" bun test --timeout 20000
run_step "Bun control-plane bundle" "${MEG_PLANE}" bun build src/index.ts --target=bun --outfile=dist/plane.js

if [[ "${SKIP_RELEASE}" -eq 0 && -x "${MEG_RS}/target/release/meg" ]]; then
  run_step "Release binary smoke (--version)" "${REPO_ROOT}" "${MEG_RS}/target/release/meg" --version
fi

if [[ -d "${MEG_EVAL}" ]] && find "${MEG_EVAL}" -name 'test_*.py' | grep -q .; then
  PY_BIN="$(command -v python3 || command -v python)"
  run_step "Python offline evals (pytest)" "${REPO_ROOT}" "${PY_BIN}" -m pytest meg-eval
fi

echo "Phase Gate check PASSED."