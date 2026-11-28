#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

for tool in cargo rustc git; do
  printf "  [OK] %-6s %s\n" "${tool}" "$("${tool}" --version | head -n 1)"
done

for rel in AGENTS.md docs/STATE.md meg-rs/Cargo.toml meg-rs/crates/meg-core/skills/tdd-discipline/SKILL.md; do
  [[ -f "${REPO_ROOT}/${rel}" ]] || { echo "Missing expected file: ${rel}" >&2; exit 1; }
  echo "  [OK] ${rel}"
done
echo "Workspace ready (100% pure Rust; no Bun/Node required)."
