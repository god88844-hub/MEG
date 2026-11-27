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

- **Single Compiled Binary (~7.5 MB):** Embedded builtin skills, DDIPE skill supply-chain scanner, local declarative plugins, native stdio + Streamable HTTP/SSE MCP host (CLI > MCP guard, `PATH` + script SHA-256 hash-pinning, ambient secret env scrubbing on spawn), and bounded read-only subagent workers (`meg worker`) all run in pure Rust (`meg-core::services`).
- **OS Sandboxing & Secret Airlock:**
  - `.meg/permissions.json` command policies (`deny` > `allow` > `ask` with fail-closed `Ask`), ambient secret env scrubbing, and `.env*` + SAL secret scrubbing across `read_file`, `grep_files`, `bash`, MCP spawns, and `@"path"` attachments.
  - Cross-OS shell confinement: Linux `bwrap` (`--unshare-pid --new-session --die-with-parent` + optional `--unshare-net`), macOS `sandbox-exec` Seatbelt, and Windows Job Objects (`ActiveProcessLimit`, `JobMemoryLimit`, UI restrictions, restricted-token probe in `strict` mode, and loopback proxy sinkhole when `network: false`).
- **Verification, LSP & Multi-Model Orchestration:**
  - Live stdio LSP write-through diagnostics on `write_file`/`edit_file`, `SealedManifest` protected-test verification with automatic restore on tamper, atomic file writes, full-range hashline integrity, `.gitignore`-aware `grep_files`, and multi-step `/undo`.
  - Diff-aware cold L3 review (`--reviewer-model`, `--pair-mode`), Delegent local-SLM subagent routing (`--delegent`, `--subagent-model ollama:qwen2.5-coder:7b`), and atomic `.meg/state.json` terminal states with deterministic exit codes (`0=GREEN`, `2=BUDGET`, `3=STALL`, `4=CHECK_CANNOT_FAIL`).
- **TUI, Headless & Custom UI Bridge:**
  - Dual Ratatui workstation (`/ui classic` and `/ui pro`), nested `AGENTS.md` / `CLAUDE.md` discovery, structured `/compact`, and Windows Credential Manager key vault (HTTPS enforced for remote endpoints).
  - Headless automation (`meg run --json`, `meg loop --check ...`) and a stateful, non-blocking, cancellable NDJSON bridge (`meg ui-bridge`) for custom frontends.

## Quickstart

```sh
# Interactive workstation
meg

# Headless task with verification gate + local-SLM subagent routing
meg run -C ./my-project --delegent --require-verify --verify-cmd "cargo test" "fix the failing parser test"

# Automated retry loop until check passes (writes .meg/state.json + exits 0/2/3/4)
meg loop -C ./my-project --check "pytest -q" --max-runs 3 "resolve test failures"
```

Default local endpoint: `http://127.0.0.1:11434/v1` (Ollama). Use `/provider` and `/model` inside the TUI to connect local servers or BYOK cloud APIs.

## Security & Scope

- **Windows non-admin confinement:** On Windows, Act-mode shell commands are bounded by Job Object process/memory/UI limits, secret env scrubbing, `.meg/permissions.json` policy enforcement, and network-off proxy sinkhole; full Windows AppContainer kernel isolation is not yet enabled for arbitrary dev toolchains. Use containers/VMs for hostile repositories.
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
