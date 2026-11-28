# MEG Build State

- **Version:** `0.2.0`
- **Active Architecture:** Pure-Rust Single Binary (`meg.exe` / `meg`) with `ServicePlane::native()` default (zero Bun/Node/DLL runtime dependency).
- **Quality Gate:** `cargo clippy --locked --no-deps --workspace -- -D warnings` (0 warnings across all 5 crates), full Rust workspace test suite across Ubuntu, macOS, and Windows, plus Python `meg-eval` offline tests and protected-path test integrity guard.

## Completed Phases

- [x] **Phase 1 (Audit Defect Remediation):** Closed P1/P2 audit defects across secret airlocks (`A01`), `meg loop` receipt/rejection integrity (`A02`, `A03`, `A05`, `A09`), finish-hook & cold L3 reviewer fail-closed logic (`A04`, `A07`), shell mutation tracking & sealed-file restore (`A06`, `D07`, `D08`), hot-file & anti-stub accuracy (`A11`, `A12`, `A13`), fail-closed bridge mode (`A14`), prior-history compaction & snapshotted multimodal attachments (`B01`, `B06`, `B07`), atomic writes, UTF-16LE reads & journal rollback (`C01`, `C03`, `C08`), provider-safe MCP naming, SSE terminal validation & Anthropic config (`E01`, `E02`, `E03`, `E05`), and runtime resilience (`F02`, `F03`, `F06`).
- [x] **Phase 2 (Monolith Splits & Pure-Rust Consolidation):** Split `meg-cli/src/main.rs`, `meg-tui/src/runtime.rs`, and `meg-core/src/shim_client.rs`; built pure-Rust `NativeServices` (`meg-core::services`) with embedded builtin skills, DDIPE scanning, plugins, native stdio/HTTP MCP host, and read-only worker delegation; verified standalone zero-Bun binary execution.
- [x] **Phase 3 (Core Trust & Editing Integrity):** Closed `C02` (full-range hashline interior line verification), `C05` (`.gitignore` support + skipped-file notes in `grep_files`), `C09` (bounded side-git timeouts + pruning), `C10` (registry JSON schema validation), `B02`/`B05` (deduped MCP schemas + nested `AGENTS.md`/`CLAUDE.md`), `B08` (quoted `@"path"` + secret airlock/scrubbing), `B04` (structured `/compact`), `D04` (local MCP binary/script SHA-256 trust pinning), `D09` (blocking remote plaintext HTTP), `F04` (bounded child stderr diagnostics), and `F05` (stateful, non-blocking, cancellable `meg ui-bridge`).
- [x] **Phase 4 (Cross-Platform CI, Installers & Benchmark Integrity):** Added 3-OS CI matrix (Windows + Linux + macOS + `pytest meg-eval`), tag-triggered release workflow with `SHA256SUMS.txt`, one-line installers (`install.ps1` / `install.sh`), isolated oracle + HTTP wire secret inspection in `run_live_bench.py` (`G02`, `G04`, `G05`), and reconciled docs (`F08`).
- [x] **Phase 5 (Daily-Driver Primitives):** Shipped native stdio LSP write-through diagnostics (`meg-tools/src/lsp.rs`) wired into `write_file`/`edit_file`, native `web_fetch` with SSRF/private-IP/redirect blocking, `.meg/permissions.json` (`deny` > `allow` > `ask`) + ambient secret env scrubbing (`meg-tools/src/sandbox.rs`), and cross-provider role routing (`provider_id:model_id` preserving Ollama `:tag` + `MEG_DISABLED_PROVIDERS` enforcement).
- [x] **Phase 6 (Diff-Aware Review, Live MCP & Sandbox Hardening):** Diff-aware cold L3 review (`6A`), live stdio MCP round-trips & deferred loading (`6B`), `PATH` + script SHA-256 MCP hash-pinning & spawn secret scrubbing (`6C-1`), and Windows Job Object / Linux `bwrap` / macOS Seatbelt hardening (`6C-2`).
- [x] **Phase 7A (Delegent Local-SLM Routing & Deterministic Terminal State):** Added `--delegent` and `--subagent-model` routing for read-only subagents, plus atomic `.meg/state.json` persistence and deterministic exit codes (`0=GREEN`, `2=BUDGET`, `3=STALL`, `4=CHECK_CANNOT_FAIL`).
- [x] **Phase 8A (Release Readiness):** Validated CI/release workflows, synced CLI/TUI flags, and cleaned `meg --help`.
- [x] **Phase 9 (3-OS Sandboxing, Stateful Daemon & Native Provider Protocols):**
  - **9A:** Linux `bwrap` `/tmp` mount order + AppArmor probe, macOS canonical Seatbelt subpaths, `mcp-trust.json` concurrency lock, and pure-Rust Unix CI parity.
  - **9B:** Real Windows `CreateRestrictedToken` + `CreateProcessAsUserW` Job-attached spawn, symlink TOCTOU guards, and persistent stateful `shell_session`.
  - **9C:** Token-authenticated loopback JSON-RPC 2.0 daemon (`meg serve`, `meg-serve/v1`) with multi-session streaming and mid-flight cancellation.
  - **9D:** Native OpenAI Responses API (`/v1/responses`) and Google Gemini native (`:streamGenerateContent?alt=sse`, `x-goog-api-key`) wire adapters with multimodal and `wiremock` suites.
- [x] **Phase 10 (Cross-OS Vault, Windows AppContainer, True PTY & Daemon Client):**
  - **10A:** Native macOS Keychain (`/usr/bin/security`), Linux Secret Service (`secret-tool`), `0600` atomic fallback vault, and 3-OS namespace isolation tests.
  - **10B:** Real Windows AppContainer (`CreateAppContainerProfile` + `SECURITY_CAPABILITIES` + workspace-scoped ACL grant + `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` + `BoundedPipeDrain`) with live outside-workspace kernel write-denial test.
  - **10C:** True OS PTY backing (`shell_session` `pty: true` via Windows ConPTY `CreatePseudoConsole` and Unix POSIX `posix_openpt`/`TIOCSCTTY`) with interactive `open`/`write`/`read`/`resize` and `isatty()` verification across all 3 OSes.
  - **10D:** Native `meg attach` daemon client (one-shot streaming + interactive REPL + loopback enforcement), fail-closed SHA-256 verification in `scripts/install.ps1` and `scripts/install.sh`, and reconciled `v0.2.0` documentation.

## Open Milestones

- [ ] **Public Same-Model Benchmarks:** Run blind, reproducible live-model benchmarks (`meg` vs. Pi / OpenCode on identical models & standard API keys across Terminal-Bench 2.0 / SWE-bench Lite) and publish raw trajectories, `pass@1`, wall-clock latency, and token cost.
