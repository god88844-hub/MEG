# MEG Architecture

MEG ships as a **standalone pure-Rust single binary (`meg` / `meg.exe`)** with zero runtime dependency on Bun, Node, or Python.

## Crate Architecture (`meg-rs/crates/`)

| Crate | Responsibility |
|---|---|
| `meg-tools` | Workspace-confined filesystem tools (`fs_tools.rs`), CRLF/UTF-16-safe atomic `edit.rs` with full-range hashline verification, `.gitignore`-aware `grep.rs`, cancellable Job-Object `bash.rs`, transactional `journal.rs` + bounded side-git snapshots, `deliverable_guard.rs` (`SealedManifest`, anti-stub & hot-frame linters), `ledger.rs`, `worktree.rs`, and schema-enforcing `registry.rs`. |
| `meg-core` | Core agent loop (`lib.rs`), modular provider transport (`shim_client/` supporting OpenAI-compatible Chat Completions & Anthropic Messages with prompt caching, SSE terminal checks, snapshotted multimodal attachments, and cancellable retries), `providers.rs` (Credential Manager + HTTPS guard), `indexer.rs`, `truncate.rs`, `finish_hook.rs`, and `services/` (pure-Rust `NativeServices`: embedded builtin skills, DDIPE scanner, plugins, native stdio/HTTP MCP host, `CLI > MCP` policy, and read-only subagent delegator). |
| `meg-tui` | Modular Ratatui workstation (`runtime/` split into events, input, media & paste; `/ui classic` and `/ui pro`; structured `/compact`; persistent sessions; model-aware context gauge). |
| `meg-cli` | Modular CLI entrypoint (`main.rs` ~86 LOC + `cli.rs`, `engine.rs`, `runner.rs`, `loop_cmd.rs`, `bridge.rs`, `tui_launch.rs`, `verify.rs`). Defaults to `ServicePlane::native()`. |
| `meg-ffi` | Optional C-ABI `cdylib` retained only for legacy Bun compatibility tests. |

## Optional / Non-Runtime Directories

- `meg-plane/` ? Legacy Bun/TypeScript control plane kept strictly as an opt-in escape hatch (`MEG_LEGACY_BUN_PLANE=1`). Not required or loaded by default.
- `meg-eval/` ? Python offline evaluation suite, scripted integration benchmarks, and live benchmark runner (`run_live_bench.py`) with isolated oracle verification and request-body secret inspection.
