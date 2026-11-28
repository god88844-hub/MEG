# MEG Session Guidance

- Read `AGENTS.md` and `docs/STATE.md` before making architectural changes.
- MEG is a 100% pure-Rust single-binary harness (`meg-rs/` with 4 crates: `meg-core`, `meg-tools`, `meg-tui`, `meg-cli`). Do not reintroduce Bun, Node, or FFI sidecar dependencies.
- Verify changes with `cargo clippy --locked --no-deps --workspace -- -D warnings` and `cargo test --locked --workspace`.
- Never delete or weaken tests under `meg-rs/crates/*/tests/` or `meg-eval/test_*.py`.
