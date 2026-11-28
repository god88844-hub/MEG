# MEG Roadmap

This roadmap is synchronized with [docs/STATE.md](STATE.md) and reflects only verified shipping code and active engineering milestones.

## Completed (`v0.1.0` -> `v0.2.0`)

- [x] **Phases 1-4:** Audit defect remediation (`A01`-`G08` core fixes), monolith splits (`meg-cli`, `meg-tui`, `meg-core`), pure-Rust single binary (`NativeServices`, zero Bun required), full-range hashline integrity, `.gitignore`-aware grep, nested `AGENTS.md`/`CLAUDE.md`, structured `/compact`, remote HTTPS guard, stateful `meg ui-bridge`, 3-OS CI, and checksummed releases.
- [x] **Phases 5-8:** Live stdio LSP write-through diagnostics, SSRF-guarded `web_fetch`, `.meg/permissions.json` (`deny` > `allow` > `ask`), cross-provider role routing, diff-aware cold L3 review, live MCP hash-pinning & deferred catalogs, Delegent local-SLM routing, and deterministic `.meg/state.json` exit codes (`0/2/3/4`).
- [x] **Phase 9:** Cross-OS sandbox hardening (Linux `bwrap`, macOS Seatbelt, Windows Restricted Token + Job Objects), symlink TOCTOU guards, loopback token-authenticated JSON-RPC 2.0 daemon (`meg serve`), and native OpenAI Responses API + Google Gemini wire adapters.
- [x] **Phase 10:** Cross-OS credential vault (Windows Credential Manager, macOS Keychain, Linux Secret Service + `0600` atomic fallback), Windows AppContainer kernel FS/network isolation, true OS PTY (`shell_session` via ConPTY / POSIX PTY), native `meg attach` daemon client, fail-closed installer checksum verification, and `v0.2.0` release reconciliation.

## Upcoming (`v0.3.0+`)

- [ ] **Public Same-Model Benchmarks:** Blind same-model benchmark against Pi and OpenCode with published raw trajectories, `pass@1`, wall-clock latency, and token cost.
- [ ] **Structural Code Intelligence & Ecosystem:** Tree-sitter AST search/edit, optional DAP debugger integration, and remote MCP OAuth support.
