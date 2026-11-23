#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CHECK_SH="${SCRIPT_DIR}/check.sh"
TARGET_REL="meg-plane/src/router.ts"
TARGET_ABS="${REPO_ROOT}/${TARGET_REL}"

if [[ -n "$(git -C "${REPO_ROOT}" status --porcelain -- "${TARGET_REL}")" ]]; then
  echo "[FAIL] ${TARGET_REL} has uncommitted changes; commit or stash before selftest." >&2
  exit 1
fi

BACKUP="$(mktemp)"
cp "${TARGET_ABS}" "${BACKUP}"
cleanup() {
  if [[ -f "${BACKUP}" ]]; then
    cp "${BACKUP}" "${TARGET_ABS}"
    rm -f "${BACKUP}"
  fi
}
trap cleanup EXIT

echo "Step 1/4: Verify clean baseline passes check.sh --fast"
bash "${CHECK_SH}" --fast

echo "Step 2/4: Plant synthetic bug delta in ${TARGET_REL}"
PY_BIN="$(command -v python3 || command -v python)"
"${PY_BIN}" - "${TARGET_ABS}" <<'PY'
import sys
p = sys.argv[1]
with open(p, "rb") as f:
    raw = f.read().decode("utf-8")
needle = "if (local) return { selected: local,"
bug = "if (local) return { selected: null, /* SELFTEST_DELTA */"
if needle not in raw:
    raise SystemExit("Selftest anchor not found")
with open(p, "wb") as f:
    f.write(raw.replace(needle, bug, 1).encode("utf-8"))
PY

echo "Step 3/4: Prove check.sh --fast fails non-zero on synthetic bug"
set +e
bash "${CHECK_SH}" --fast
BUG_CODE=$?
set -e

echo "Step 4/4: Revert synthetic bug delta"
cleanup

if [[ "${BUG_CODE}" -eq 0 ]]; then
  echo "[FAIL] Selftest FAILED: check.sh exited 0 despite planted bug delta!" >&2
  exit 1
fi
echo "[OK]   check.sh caught synthetic bug delta (exit ${BUG_CODE})"

if [[ -n "$(git -C "${REPO_ROOT}" status --porcelain -- "${TARGET_REL}")" ]]; then
  echo "[FAIL] ${TARGET_REL} remained modified after revert!" >&2
  exit 1
fi

bash "${CHECK_SH}" --fast
echo "Selftest PASSED (Green -> Red on delta -> Clean Revert -> Green)."