# MEG Roadmap

This roadmap is synchronized with [docs/STATE.md](STATE.md) and reflects only verified shipping code and active engineering milestones.

## Completed (v0.1.x -> v0.2 Reset Foundation)

- [x] **Phase 1:** Core security, verification, compaction, SSE, MCP naming, and editing defect remediation.
- [x] **Phase 2:** Modularized `meg-cli`, `meg-tui`, and `meg-core`; replaced mandatory Bun sidecar with pure-Rust `NativeServices` single binary.
- [x] **Phase 3:** Full-range hashline integrity (`C02`), `.gitignore` grep (`C05`), registry schema enforcement (`C10`), nested `AGENTS.md`/`CLAUDE.md` (`B05`), structured `/compact` (`B04`), MCP executable hashing (`D04`), remote HTTPS guard (`D09`), and stateful cancellable `meg ui-bridge` (`F05`).
- [x] **Phase 4:** Cross-platform CI (Windows/Linux/macOS + Python evals), checksummed releases, one-line installers, isolated benchmark oracles + wire secret inspection (`G04`/`G05`), and reconciled docs.

## Upcoming

- [ ] **Phase 5 ? Daily-Driver Engineering Primitives**
  - [ ] Native LSP client wired into `edit_file` / `write_file` for instant compiler/type diagnostics.
  - [ ] OS-level Act-mode sandboxing (Windows Restricted Token/AppContainer, Linux Landlock/`bwrap`, macOS Seatbelt) separated from `allow / ask / deny` approvals.
  - [ ] Native `web_fetch` tool, deferred MCP tool loading, and independent cross-provider role routing.
- [ ] **Phase 6 ? Daemon, PTY & Public Benchmarks**
  - [ ] Stateful JSON-RPC server (`meg serve`) and persistent interactive PTY shell tool.
  - [ ] Blind same-model benchmark against Pi and OpenCode with published raw trajectories, `pass@1`, and token cost.
