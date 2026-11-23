# CLAUDE.md

Read and follow `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/PLAN.md`, and `docs/STATE.md` before making any edits.

## Non-Negotiable Constraints
- `meg-rs/crates/meg-tui/` is **FROZEN** until Phase 8.
- Follow the exact micro-task order in `docs/PLAN.md` (currently tracked in `docs/STATE.md`).
- Run ONLY the Tier-1 Micro-Check per TODO; run the full Tier-2 suite at Phase Exit.
- Keep `meg-rs/Cargo.lock` in sync so `cargo test --locked --workspace` passes in CI.
- Never delete or weaken tests under `tests/` or `meg-plane/test/`.
- Verify every claim against actual command output.