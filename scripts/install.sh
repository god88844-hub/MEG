#!/usr/bin/env sh
# MEG Pure-Rust Single-Binary Installer (Linux x64 / macOS ARM64)
# Usage: curl -fsSL https://raw.githubusercontent.com/god88844-hub/MEG/main/scripts/install.sh | sh
set -eu

REPO="god88844-hub/MEG"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}:${ARCH}" in
  Linux:x86_64)  ASSET="meg-x86_64-unknown-linux-gnu" ;;
  Darwin:arm64)  ASSET="meg-aarch64-apple-darwin" ;;
  *)
    echo "Unsupported platform ${OS}/${ARCH}. Build from source: cd meg-rs && cargo build --locked --release -p meg-cli" >&2
    exit 1
    ;;
esac

TAG="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
if [ -z "${TAG}" ]; then
  echo "Could not resolve latest release tag for ${REPO}" >&2
  exit 1
fi

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "${INSTALL_DIR}"
TMP_BIN="$(mktemp)"
trap 'rm -f "${TMP_BIN}"' EXIT

echo "Downloading MEG ${TAG} (${ASSET})..."
curl -fsSL "https://github.com/${REPO}/releases/download/${TAG}/${ASSET}" -o "${TMP_BIN}"

if SUMS="$(curl -fsSL "https://github.com/${REPO}/releases/download/${TAG}/SHA256SUMS.txt" 2>/dev/null)"; then
  EXPECTED="$(printf '%s\n' "${SUMS}" | awk -v a="${ASSET}" '$2 == a {print $1; exit}')"
  if [ -n "${EXPECTED}" ]; then
    if command -v sha256sum >/dev/null 2>&1; then
      ACTUAL="$(sha256sum "${TMP_BIN}" | awk '{print $1}')"
    else
      ACTUAL="$(shasum -a 256 "${TMP_BIN}" | awk '{print $1}')"
    fi
    if [ "${ACTUAL}" != "${EXPECTED}" ]; then
      echo "SHA-256 mismatch! expected=${EXPECTED} actual=${ACTUAL}" >&2
      exit 1
    fi
    echo "Verified SHA-256: ${ACTUAL}"
  fi
fi

chmod +x "${TMP_BIN}"
mv "${TMP_BIN}" "${INSTALL_DIR}/meg"
echo "Installed MEG ${TAG} -> ${INSTALL_DIR}/meg"
"${INSTALL_DIR}/meg" --version
