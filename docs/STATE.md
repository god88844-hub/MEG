# MEG Build State

- **Active Architecture:** Pure-Rust Single Binary (`meg.exe` / `meg`, ~7.55 MB) with `ServicePlane::native()` default (zero Bun/DLL runtime dependency).
- **Quality Gate:** `cargo clippy --locked --no-deps --workspace -- -D warnings` (0 warnings across all 5 crates), full Rust workspace tests, Python `meg-eval` suite, and cross-platform CI (Windows, Linux, macOS).

## Completed Hardening & Reset Phases

- [x] **Phase 1 (Audit Defect Remediation):** Closed P1/P2 audit defects across secret airlocks (`A01`), `meg loop` receipt/rejection integrity (`A02`, `A03`, `A05`, `A09`), finish-hook & cold L3 reviewer fail-closed logic (`A04`, `A07`), shell mutation tracking & sealed-file restore (`A06`, `D07`, `D08`), hot-file & anti-stub accuracy (`A11`, `A12`, `A13`), fail-closed bridge mode (`A14`), prior-history compaction & snapshotted multimodal attachments (`B01`, `B06`, `B07`), atomic writes, UTF-16LE reads & journal rollback (`C01`, `C03`, `C08`), provider-safe MCP naming, SSE terminal validation & Anthropic config (`E01`, `E02`, `E03`, `E05`), and runtime resilience (`F02`, `F03`, `F06`).
- [x] **Phase 2 (Monolith Splits & Pure-Rust Consolidation):** Split `meg-cli/src/main.rs` (3,374 -> 86 LOC), `meg-tui/src/runtime.rs`, and `meg-core/src/shim_client.rs`; built pure-Rust `NativeServices` (`meg-core::services`) with embedded builtin skills, DDIPE scanning, plugins, native stdio/HTTP MCP host, and read-only worker delegation; verified standalone zero-Bun binary execution.
- [x] **Phase 3 (Remaining Core Trust Defects):** Closed `C02` (full-range hashline interior line verification), `C05` (`.gitignore` support + skipped-file notes in `grep_files`), `C09` (bounded side-git timeouts + pruning), `C10` (registry JSON schema validation), `B02`/`B05` (deduped MCP schemas + nested `AGENTS.md`/`CLAUDE.md`), `B08` (quoted `@"path"` + secret airlock/scrubbing), `B04` (structured `/compact`), `D04` (local MCP binary/script SHA-256 trust pinning), `D09` (blocking remote plaintext HTTP), `F04` (bounded child stderr diagnostics), and `F05` (stateful, non-blocking, cancellable `meg ui-bridge`).
- [x] **Phase 4 (Cross-Platform CI, Installers, Benchmark Integrity & Docs Reconciliation):** Added 3-OS CI matrix (Windows + Linux + macOS + `pytest meg-eval`), tag-triggered release workflow with `SHA256SUMS.txt`, one-line installers (`install.ps1` / `install.sh`), isolated oracle + HTTP wire secret inspection in `run_live_bench.py` (`G02`, `G04`, `G05`), and reconciled all project documentation (`F08`).

## Next Active Phases

- [ ] **Phase 5 ? Core Daily-Driver Primitives:**
  - Native LSP write-through diagnostics (`rust-analyzer`, `typescript-language-server`, `pyright`, `gopls`) wired into `edit_file` / `write_file`.
  - Real OS-level sandboxing for Act mode (Windows Restricted Token/AppContainer, Linux Landlock/`bwrap`, macOS Seatbelt) + granular `allow / ask / deny` permission rules and env-var secret stripping.
  - Native `web_fetch` tool, deferred MCP tool loading, and independent cross-provider role routing (`E06`).
- [ ] **Phase 6 ? Multi-Surface Daemon & Live Benchmark Proof:**
  - Versioned stateful JSON-RPC daemon (`meg serve`) + interactive PTY execution.
  - Public, reproducible same-model benchmark (`meg` vs Pi / OpenCode) with raw traces and cost/token breakdown.
