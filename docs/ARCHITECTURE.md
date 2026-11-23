# MEG Architecture

MEG is a three-layer polyglot coding-agent harness designed for local Windows/POSIX workstations and offline evaluation.

## Layer Overview

| Layer | Directory | Language / Runtime | Responsibility |
| :--- | :--- | :--- | :--- |
| **Ring-0 Native Kernel** | `meg-rs/` | Rust (GNU toolchain on Windows) | Deterministic tools, workspace confinement, OS Job Objects / sandboxing, FFI hot path, native streaming provider client, agent loop, CLI/TUI binary (`meg.exe`). |
| **Layer-2 Control Plane** | `meg-plane/` | TypeScript on Bun | JSON-RPC 2.0 over stdio, `bun:ffi` into `meg_ffi.dll`, model routing, persistent session trees, context compilation, memory, `SKILL.md` progressive disclosure, MCP host, verification gauntlet, multi-agent orchestration. |
| **Offline Eval & Evolution** | `meg-eval/` | Python 3.11+ (`pytest`) | Strictly offline 4-layer evals, domain oracles, reliability metrics, label-free test synthesis, and PR-only gated evolution (introduced in Phases 6-7). |

## Current Crate & Module Layout

### `meg-rs/crates/`
- `meg-tools` -- Workspace-confined filesystem tools (`fs_tools.rs`), CRLF-safe `edit.rs`, regex `grep.rs`, cancellable Job-Object `bash.rs`, multi-step `journal.rs`, `deliverable_guard.rs`, JSON-schema `registry.rs`.
- `meg-core` -- Agent loop (`lib.rs`), streaming OpenAI/Anthropic/Ollama client (`shim_client.rs`) with native tool calling, Anthropic `cache_control`, and cancellable retries, `providers.rs` (OS Credential Manager + `Zeroize`), `provider_catalog.rs`, `indexer.rs`, `truncate.rs`, `json_repair.rs`, `finish_hook.rs`, `plane_client.rs`, `cancel.rs`.
- `meg-ffi` -- C-ABI `cdylib` (`meg_ffi.dll`) exposing search, grep, list_dir, read, edit, truncate, parse, and workspace indexing to Bun with explicit `meg_free`.
- `meg-cli` -- `meg` binary (`tui`, headless `run`, read-only `worker`).
- `meg-tui` -- Interactive Ratatui workstation (**FROZEN until Phase 8**).

### `meg-plane/src/`
- `index.ts` -- Multiplexed JSON-RPC 2.0 stdio dispatch server.
- `ffi-bridge.ts` -- `bun:ffi` bindings to `meg_ffi.dll`.
- `router.ts` -- Local/cloud model router with honest `providers.json` sync.
- `session-tree.ts` -- Forkable session tree with atomic persistence under `.meg/sessions/`.
- `agents.ts` -- Native worker delegation manager with Windows `taskkill /T /F` cleanup and structured events.
- `extensions.ts` & `mcp-host.ts` -- Declarative skills/plugins loader and official MCP SDK host (stdio, Streamable HTTP, SSE).
- `security-router.ts` -- Direct security/crypto root-thread router and hand-off cap.

## Trust & Safety Boundaries
- Credentials stay in OS Credential Manager (`MEG/provider/<id>`, scoped per `MEG_CONFIG_DIR`) or process env vars; never in JSON configs, URLs, or session files.
- HTTP clients disable redirects to prevent credential leakage across hosts.
- Delegated workers run in read-only Plan mode without nested delegation.
- `meg-tui/` stays untouched through Phases 0-7 so all kernel, control-plane, and eval upgrades land against a stable UI contract.