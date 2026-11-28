#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
MEG_RS="${REPO_ROOT}/meg-rs"

if [[ "${1:-}" == "--fast" ]]; then
  echo "==> Rust fast check (meg-core)"
  (cd "${MEG_RS}" && cargo test --locked -p meg-core service_plane_is_always_pure_rust_native)
  echo "[OK] Fast check passed."
  exit 0
fi

echo "==> Strict zero-warning clippy"
(cd "${MEG_RS}" && cargo clippy --locked --no-deps --workspace -- -D warnings)
echo "==> Rust workspace tests (--locked)"
(cd "${MEG_RS}" && cargo test --locked --workspace)
if command -v pytest >/dev/null 2>&1 || python3 -m pytest --version >/dev/null 2>&1; then
  echo "==> Python offline eval suite"
  (cd "${REPO_ROOT}" && python3 -m pytest meg-eval -q)
fi
echo "[OK] Full check passed."
