# MEG

A fast, **pure-Rust single-binary** terminal coding-agent harness for local models (Ollama, LM Studio, llama.cpp, vLLM) and cloud BYOK APIs (OpenAI Chat & Responses API, Anthropic Messages, Google Gemini native, OpenAI-compatible). No MEG account, no telemetry, and **no Bun or Node runtime required**.

## Install

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/god88844-hub/MEG/main/scripts/install.ps1 | iex
```

**Linux (x64) / macOS (ARM64):**
```sh
curl -fsSL https://raw.githubusercontent.com/god88844-hub/MEG/main/scripts/install.sh | sh
```

Both installers verify `SHA256SUMS.txt` fail-closed before moving the binary into `~/.local/bin`.

**Build from source (only Rust required):**
```sh
cd meg-rs
cargo build --locked --release -p meg-cli
./target/release/meg --version
```

## Core Capabilities

- **Single Compiled Binary (~8.5 MB):** Embedded builtin skills, DDIPE skill supply-chain scanner, local declarative plugins, native stdio + Streamable HTTP/SSE MCP host (CLI > MCP guard, `PATH` + script SHA-256 hash-pinning, ambient secret env scrubbing on spawn), and bounded read-only subagent workers (`meg worker`) all run in pure Rust (`meg-core::services`).
- **Native Multi-Protocol Provider Layer:**
  - Dedicated wire adapters for **OpenAI Chat Completions**, **OpenAI Responses API** (`/v1/responses`), **Anthropic Messages** (prompt caching + reasoning effort), **Google Gemini native** (`:streamGenerateContent?alt=sse` with `x-goog-api-key` header auth), and local **Ollama / OpenAI-compatible** servers.
  - Cross-OS credential vault: **Windows Credential Manager**, **macOS Keychain** (`/usr/bin/security`), and **Linux Secret Service** (`secret-tool`) with a `0600` atomic fallback file - keys are never written to `providers.json` or session transcripts.
- **OS-Level Sandboxing & Secret Airlock:**
  - `.meg/permissions.json` command policies (`deny` > `allow` > `ask` with fail-closed `Ask`), ambient secret env scrubbing, symlink TOCTOU guards, and `.env*` + SAL secret scrubbing across `read_file`, `grep_files`, `bash`, `shell_session`, MCP spawns, and `@"path"` attachments.
  - Real OS containment in `MEG_SANDBOX=strict`: **Windows AppContainer** kernel FS/network ACL isolation (scoped strictly to the workspace) + Restricted Token + Job Object limits, **Linux `bwrap`** (`--unshare-pid --new-session --die-with-parent` + `--unshare-net` + AppArmor probe), and **macOS `sandbox-exec`** Seatbelt profiles.
- **Verification, LSP, PTY & Multi-Model Orchestration:**
  - Live stdio **LSP write-through diagnostics** on `write_file`/`edit_file`, true OS **PTY** backing (`shell_session` with Windows **ConPTY** and Unix **POSIX PTY** `open`/`write`/`read`/`resize`/`exec`), `SealedManifest` protected-test verification with automatic restore on tamper, atomic file writes, full-range hashline integrity, `.gitignore`-aware `grep_files`, native `web_fetch` (with SSRF protection), and multi-step `/undo`.
  - Diff-aware cold L3 review (`--reviewer-model`, `--pair-mode`), Delegent local-SLM subagent routing (`--delegent`, `--subagent-model ollama:qwen2.5-coder:7b`), and atomic `.meg/state.json` terminal states with deterministic exit codes (`0=GREEN`, `2=BUDGET`, `3=STALL`, `4=CHECK_CANNOT_FAIL`).
- **TUI, Headless Daemon & Client (`meg serve` / `meg attach`):**
  - Dual Ratatui workstation (`/ui classic` and `/ui pro`), nested `AGENTS.md` / `CLAUDE.md` discovery, and structured `/compact`.
  - Token-authenticated loopback JSON-RPC 2.0 daemon (`meg serve`, `meg-serve/v1`), native client (`meg attach` for one-shot streaming or interactive REPL), headless runners (`meg run --json`, `meg loop --check ...`), and non-blocking NDJSON `meg ui-bridge`.

## Quickstart

```sh
# Interactive workstation
meg

# Headless task with verification gate + local-SLM subagent routing
meg run -C ./my-project --delegent --require-verify --verify-cmd "cargo test" "fix the failing parser test"

# Automated retry loop until check passes (writes .meg/state.json + exits 0/2/3/4)
meg loop -C ./my-project --check "pytest -q" --max-runs 3 "resolve test failures"

# Headless daemon + native client attach
meg serve --port 41234 -C ./my-project
meg attach --addr 127.0.0.1:41234 --token "<token>" "summarize recent changes"
```

Default local endpoint: `http://127.0.0.1:11434/v1` (Ollama). Use `/provider` and `/model` inside the TUI to connect local servers or BYOK cloud APIs.
