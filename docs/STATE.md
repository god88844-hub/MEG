# MEG Build State

- **Version:** `0.2.0`
- **Active Architecture:** Pure-Rust Single Binary (`meg.exe` / `meg`) with `ServicePlane::native()` default (zero Bun/Node/DLL runtime dependency).
- **Quality Gate:** `cargo clippy --locked --no-deps --workspace -- -D warnings` (0 warnings across all 5 crates), 459 Rust workspace tests, 26 Python `meg-eval` offline tests, and 3-OS CI (Windows, Linux, macOS).

## Completed Phases

- [x] **Phase 1 (Audit Defect Remediation):** Closed P1/P2 audit defects across secret airlocks (`A01`), `meg loop` receipt/rejection integrity (`A02`, `A03`, `A05`, `A09`), finish-hook & cold L3 reviewer fail-closed logic (`A04`, `A07`), shell mutation tracking & sealed-file restore (`A06`, `D07`, `D08`), hot-file & anti-stub accuracy (`A11`, `A12`, `A13`), fail-closed bridge mode (`A14`), prior-history compaction & snapshotted multimodal attachments (`B01`, `B06`, `B07`), atomic writes, UTF-16LE reads & journal rollback (`C01`, `C03`, `C08`), provider-safe MCP naming, SSE terminal validation & Anthropic config (`E01`, `E02`, `E03`, `E05`), and runtime resilience (`F02`, `F03`, `F06`).
- [x] **Phase 2 (Monolith Splits & Pure-Rust Consolidation):** Split `meg-cli/src/main.rs`, `meg-tui/src/runtime.rs`, and `meg-core/src/shim_client.rs`; built pure-Rust `NativeServices` (`meg-core::services`) with embedded builtin skills, DDIPE scanning, plugins, native stdio/HTTP MCP host, and read-only worker delegation; verified standalone zero-Bun binary execution.
- [x] **Phase 3 (Core Trust & Editing Integrity):** Closed `C02` (full-range hashline interior line verification), `C05` (`.gitignore` support + skipped-file notes in `grep_files`), `C09` (bounded side-git timeouts + pruning), `C10` (registry JSON schema validation), `B02`/`B05` (deduped MCP schemas + nested `AGENTS.md`/`CLAUDE.md`), `B08` (quoted `@"path"` + secret airlock/scrubbing), `B04` (structured `/compact`), `D04` (local MCP binary/script SHA-256 trust pinning), `D09` (blocking remote plaintext HTTP), `F04` (bounded child stderr diagnostics), and `F05` (stateful, non-blocking, cancellable `meg ui-bridge`).
- [x] **Phase 4 (Cross-Platform CI, Installers & Benchmark Integrity):** Added 3-OS CI matrix (Windows + Linux + macOS + `pytest meg-eval`), tag-triggered release workflow with `SHA256SUMS.txt`, one-line installers (`install.ps1` / `install.sh`), isolated oracle + HTTP wire secret inspection in `run_live_bench.py` (`G02`, `G04`, `G05`), and reconciled docs (`F08`).
- [x] **Phase 5 (Daily-Driver Primitives):** Shipped native stdio LSP write-through diagnostics (`meg-tools/src/lsp.rs`) wired into `write_file`/`edit_file`, native `web_fetch` with SSRF/private-IP/redirect blocking, `.meg/permissions.json` (`deny` > `allow` > `ask`) + ambient secret env scrubbing (`meg-tools/src/sandbox.rs`), and cross-provider role routing (`provider_id:model_id` preserving Ollama `:tag` + `MEG_DISABLED_PROVIDERS` enforcement).
- [x] **Phase 6 (Diff-Aware Review, Live MCP & Sandbox Hardening):**
  - **6A:** Diff-aware cold L3 review & mutation tracking.
  - **6B:** Live stdio MCP round-trips & deferred loading for large MCP tool catalogs.
  - **6C-1:** Full `PATH` + script SHA-256 hash-pinning (`extensions.rs`) and ambient host secret scrubbing on MCP stdio spawn (`mcp.rs`).
  - **6C-2:** Windows Job Object process/RAM/UI limits (`ActiveProcessLimit`, `JobMemoryLimit`, `DIE_ON_UNHANDLED_EXCEPTION`, restricted-token probe in `strict` mode), loopback proxy sinkhole when `network: false`, and Linux `bwrap` `--unshare-pid --new-session` tightening.
- [x] **Phase 7A (Delegent Local-SLM Routing & Deterministic Terminal State):** Added `--delegent` and `--subagent-model` routing (role env -> explicit spec -> enabled `ollama`/`lmstudio` preset) for read-only subagents, plus atomic `.meg/state.json` persistence and deterministic exit codes (`0=GREEN`, `2=BUDGET`, `3=STALL`, `4=CHECK_CANNOT_FAIL`) across `meg run` and `meg loop`.
- [x] **Phase 8A (Release Readiness):** Fixed `.github/workflows/{ci,release}.yml` YAML indentation, forwarded `--delegent`/`--subagent-model` in TUI launch, cleaned `meg --help`, made `scripts/install.ps1` SHA-256 verification fail-closed, and synchronized `README.md`, `SECURITY.md`, and `docs/`.

## Next Active Phases

- [ ] **Phase 9 ? Multi-Surface Daemon & Interactive PTY:**
  - Versioned stateful JSON-RPC daemon (`meg serve`) and persistent interactive PTY shell execution.
  - Optional Windows AppContainer isolation for untrusted repositories.
- [ ] **Phase 10 ? Public Same-Model Benchmarks:**
  - Blind, reproducible same-model benchmark (`meg` vs Pi / OpenCode) with published raw trajectories, `pass@1`, and token/cost breakdown.
