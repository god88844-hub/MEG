# MEG Engineering Rules

## Architecture (100% Pure Rust)

- `meg-rs/` -- 4-crate pure-Rust workspace (`meg-core`, `meg-tools`, `meg-tui`, `meg-cli`) compiling to a single standalone binary (`meg.exe` / `meg`) with zero Node, Bun, Python, or external DLL runtime dependencies.
- `meg-rs/crates/meg-core/skills/` -- Embedded builtin `SKILL.md` files compiled directly into the binary.
- `meg-eval/` -- Standalone Python evaluation runner, metrics, and isolated benchmark oracles (dev/eval only; never required at runtime).
- `docs/STATE.md` & `docs/ROADMAP.md` -- Synchronized build state and verified milestones.

## Verification & Quality Gates

1. **Zero-warning Clippy:** `cargo clippy --locked --no-deps --workspace -- -D warnings` must pass before every commit.
2. **Locked workspace tests:** `cargo test --locked --workspace` and `python -m pytest meg-eval -q` must pass before merging or tagging.
3. **Protected paths:** Never delete or weaken tests under `meg-rs/crates/*/tests/`, `meg-eval/test_*.py`, or `meg-eval/oracles/test_*.py` to make a check pass.
4. **Fail-closed security:** Keep secret airlocks (`.env*`, SAL scrubbers, credential vault isolation, remote HTTP guard, OS sandboxing) strictly fail-closed.
