# MEG Roadmap

This roadmap is synchronized with [docs/STATE.md](STATE.md) and reflects only verified shipping code and active engineering milestones.

## Completed (`v0.1.0` -> `v0.2.0`)

- [x] **Phase 1:** Core security, verification, compaction, SSE, MCP naming, and editing defect remediation.
- [x] **Phase 2:** Modularized `meg-cli`, `meg-tui`, and `meg-core`; replaced mandatory Bun sidecar with pure-Rust `NativeServices` single binary.
- [x] **Phase 3:** Full-range hashline integrity (`C02`), `.gitignore` grep (`C05`), registry schema enforcement (`C10`), nested `AGENTS.md`/`CLAUDE.md` (`B05`), structured `/compact` (`B04`), remote HTTPS guard (`D09`), and stateful cancellable `meg ui-bridge` (`F05`).
- [x] **Phase 4:** Cross-platform CI (Windows/Linux/macOS + Python evals), checksummed releases, one-line installers, isolated benchmark oracles + wire secret inspection (`G04`/`G05`), and reconciled docs.
- [x] **Phase 5:** Native LSP write-through diagnostics (`lsp.rs`), native `web_fetch` with SSRF guard, `.meg/permissions.json` (`deny` > `allow` > `ask`), and cross-provider role routing (`provider:model`).
- [x] **Phase 6:** Diff-aware cold L3 review (`6A`), live MCP round-trips & deferred catalog loading (`6B`), `PATH` + script SHA-256 MCP hash-pinning & spawn secret scrubbing (`6C-1`), and cross-OS sandbox / Windows Job Object resource + UI + network-sinkhole hardening (`6C-2`).
- [x] **Phase 7A:** Delegent local-SLM subagent routing (`--delegent`, `--subagent-model`) and atomic `.meg/state.json` deterministic exits (`0=GREEN`, `2=BUDGET`, `3=STALL`, `4=CHECK_CANNOT_FAIL`).
- [x] **Phase 8A:** Validated CI/release workflows, fail-closed installer checksum checks, synced CLI/TUI flags, and reconciled release documentation.

## Upcoming

- [ ] **Phase 9 ? Daemon, PTY & Deeper Windows Isolation**
  - [ ] Stateful JSON-RPC server (`meg serve`) and persistent interactive PTY shell tool.
  - [ ] Opt-in Windows AppContainer profile for untrusted/hostile workspaces.
- [ ] **Phase 10 ? Public Same-Model Benchmarks**
  - [ ] Blind same-model benchmark against Pi and OpenCode with published raw trajectories, `pass@1`, and token cost.
