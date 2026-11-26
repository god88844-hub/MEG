# MEG

A fast, **pure-Rust single-binary** terminal coding-agent harness for local models (Ollama, LM Studio, llama.cpp, vLLM) and cloud BYOK APIs (Anthropic Messages, OpenAI-compatible). No MEG account, no telemetry, and **no Bun or Node runtime required**.

## Install

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/god88844-hub/MEG/main/scripts/install.ps1 | iex
```

**Linux (x64) / macOS (ARM64):**
```sh
curl -fsSL https://raw.githubusercontent.com/god88844-hub/MEG/main/scripts/install.sh | sh
```

**Build from source (only Rust required):**
```sh
cd meg-rs
cargo build --locked --release -p meg-cli
./target/release/meg --version
```

## Core Capabilities

- **Single Compiled Binary (~7.5 MB):** Embedded builtin skills (10 packs), DDIPE skill supply-chain scanner, local declarative plugins, native stdio + Streamable HTTP/SSE MCP host (CLI > MCP guard, trust pinning by file SHA-256), and bounded read-only subagent workers (`meg worker`) all run in pure Rust (`meg-core::services`).
- **Safety & Verification Guarantees:**
  - Consistent `.env*` secret airlock and SAL secret scrubbing across `read_file`, `grep_files`, `bash`, and `@"path"` text attachments.
  - Strict Plan (read-only) vs Act/Build execution gating, `SealedManifest` protected-test verification with automatic restore on tamper, atomic file writes, full-range hashline integrity (`range_hash` / `line_hashes`), `.gitignore`-aware `grep_files`, and multi-step file-mutation `/undo`.
  - Honest verification receipts: `--require-verify` and `--verify-cmd` detect real verification commands and shell mutations without substring decoys, and `meg loop` propagates inner rejections accurately.
- **TUI, Headless & Custom UI Bridge:**
  - Dual Ratatui workstation (`/ui classic` and `/ui pro`), nested `AGENTS.md` / `CLAUDE.md` discovery, structured `/compact`, and Windows Credential Manager key vault (HTTPS enforced for remote endpoints).
  - Headless automation (`meg run --json`, `meg loop --check ...`) and a stateful, non-blocking, cancellable NDJSON bridge (`meg ui-bridge`) for custom frontends.

## Quickstart

```sh
# Interactive workstation
meg

# Headless task with verification gate
meg run -C ./my-project --require-verify --verify-cmd "cargo test" "fix the failing parser test"

# Automated retry loop until check passes
meg loop -C ./my-project --check "pytest -q" --max-runs 3 "resolve test failures"
```

Default local endpoint: `http://127.0.0.1:11434/v1` (Ollama). Use `/provider` and `/model` inside the TUI to connect local servers or BYOK cloud APIs.

## Security & Current Limitations (Honest Scope)

- **Act mode is not yet an OS kernel sandbox.** Native file tools confine paths to the workspace and Plan mode / delegated workers are read-only, but shell commands in Act mode run with your user privileges (OS sandboxing ? Windows Restricted Token/AppContainer, Linux Landlock/`bwrap`, macOS Seatbelt ? is scheduled for Phase 5).
- **LSP write-through diagnostics** are not yet wired into `edit_file` (Phase 5).
- **Legacy Bun control plane:** `meg-plane/` remains in the repository strictly as an opt-in legacy compatibility escape hatch (`MEG_LEGACY_BUN_PLANE=1`). Normal `meg` operation never invokes Bun or `meg_ffi.dll`.
- See [SECURITY.md](SECURITY.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), and [docs/STATE.md](docs/STATE.md) for verified scope and boundaries.

## Verification

```powershell
cd meg-rs
cargo clippy --locked --no-deps --workspace -- -D warnings
cargo test --locked --workspace

python -m pytest meg-eval -q
```

## License

MIT. See [LICENSE](LICENSE).
