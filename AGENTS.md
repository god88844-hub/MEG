# MEG Agent Operating Contract

Single source of truth for building `god88844-hub/MEG`.

## Hard Rules
1. **Plan lock**: Execute tasks strictly in order from `docs/PLAN.md`. Track the single active task in `docs/STATE.md`.
2. **Freeze rule**: `meg-rs/crates/meg-tui/` is FROZEN until Phase 8. Never edit files under `meg-tui/` in Phases 0-7.
3. **No misleading claims**: Never mark a TODO `[x]` without running its Micro-Check and observing passing output.
4. **Testing cadence**:
   - **Tier-1 Micro-Check** (2-10s): Run ONLY the targeted test listed under the current TODO.
   - **Tier-2 Phase Gate**: Run full `cargo test --locked --workspace`, `cargo build --locked --release --workspace`, `bun test`, and (from Phase 7) `pytest` ONLY at Phase Exit.
5. **Lockfile discipline**: Any `Cargo.toml` change must update and commit `meg-rs/Cargo.lock` so `--locked` stays green in CI.
6. **Protected paths**: Never delete or weaken tests under `tests/` or `meg-plane/test/` to make a check pass.
7. **Windows 10 / PS 5.1 safety**: All scripts and file operations must work cleanly on Windows 10 PowerShell 5.1 as well as POSIX shells.
8. **Offline-first**: Tests must never require live external API keys or network calls.
9. **No silent secret leaks**: Never store keys in JSON configs, URLs, logs, or git history.

## Repository Map
- `meg-rs/` -- Rust Ring-0 kernel (`meg-core`, `meg-tools`, `meg-ffi`, `meg-cli`, frozen `meg-tui`).
- `meg-plane/` -- Bun/TypeScript control plane (JSON-RPC 2.0 over stdio, FFI bridge, MCP, skills, orchestration).
- `meg-eval/` -- Python 3.11+ offline evaluation, domain oracles, and PR-only gated evolution (Phase 7).
- `docs/PLAN.md` -- Living Master Plan (v1.2).
- `docs/STATE.md` -- Current task pointer, last verified commit, and session handoff state.
- `docs/ARCHITECTURE.md` -- Layer boundaries and current vs. target module map.
- `docs/API_INDEX.md` -- Public API inventory for `meg-rs` and `meg-plane` (created in `P0-04`).

## Standard Session Loop
1. Read `docs/STATE.md` and the active TODO in `docs/PLAN.md`.
2. Implement ONLY that micro-task.
3. Run its exact Tier-1 Micro-Check.
4. Update `docs/PLAN.md` (`[x]`) and `docs/STATE.md`, then commit and push.