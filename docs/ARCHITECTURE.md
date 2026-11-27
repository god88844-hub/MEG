# MEG Architecture

MEG ships as a **standalone pure-Rust single binary (`meg` / `meg.exe`)** with zero runtime dependency on Bun, Node, or Python.

## Crate Architecture (`meg-rs/crates/`)

| Crate | Responsibility |
|---|---|
| `meg-tools` | Workspace-confined filesystem tools (`fs_tools.rs`), CRLF/UTF-16-safe atomic `edit.rs` with full-range hashline verification, `.gitignore`-aware `grep.rs`, live stdio write-through `lsp.rs`, OS-confined `sandbox.rs` + `bash.rs` (`.meg/permissions.json`, secret env scrubbing, Linux `bwrap` / macOS Seatbelt / Windows Job Object process+RAM+UI limits & network-off sinkhole), transactional `journal.rs` + bounded side-git snapshots, `deliverable_guard.rs` (`SealedManifest`, anti-stub & hot-frame linters), `ledger.rs`, `worktree.rs`, and schema-enforcing `registry.rs`. |
| `meg-core` | Core agent loop (`lib.rs`), modular provider transport (`shim_client/` supporting OpenAI-compatible Chat Completions & Anthropic Messages with prompt caching, SSE terminal checks, snapshotted multimodal attachments, and cancellable retries), `providers.rs` (Credential Manager + HTTPS guard + cross-provider role routing), SSRF-safe `web_fetch.rs`, `indexer.rs`, `truncate.rs`, `finish_hook.rs`, and `services/` (pure-Rust `NativeServices`: embedded builtin skills, DDIPE scanner, plugins, native stdio/HTTP MCP host with `PATH`+script SHA-256 hash-pinning & spawn secret scrubbing, `CLI > MCP` policy, and Delegent local-SLM read-only subagent delegator). |
| `meg-tui` | Modular Ratatui workstation (`runtime/` split into events, input, media & paste; `/ui classic` and `/ui pro`; structured `/compact`; persistent sessions; model-aware context gauge). |
| `meg-cli` | Modular CLI entrypoint (`main.rs` + `cli.rs`, `engine.rs`, `runner.rs`, `loop_cmd.rs`, `bridge.rs`, `tui_launch.rs`, `verify.rs`). Defaults to `ServicePlane::native()`, writes atomic `.meg/state.json`, and exits with deterministic codes (`0=GREEN`, `2=BUDGET`, `3=STALL`, `4=CHECK_CANNOT_FAIL`). |
| `meg-ffi` | Optional C-ABI `cdylib` retained only for legacy Bun compatibility tests. |

## Optional / Non-Runtime Directories

- `meg-plane/` ? Legacy Bun/TypeScript control plane kept strictly as an opt-in escape hatch (`MEG_LEGACY_BUN_PLANE=1`). Not required or loaded by default.
- `meg-eval/` ? Python offline evaluation suite, scripted integration benchmarks, and live benchmark runner (`run_live_bench.py`) with isolated oracle verification and request-body secret inspection.
