# MEG Harness Upgrade Roadmap

Single source of truth for upgrading MEG's native Rust core (`meg-rs`) and Bun/TypeScript control plane (`meg-plane`).

## Ground Rules
- **`meg-tui` stays frozen until Phase 8** (no UI/layout churn while hardening core engine, tools, FFI, and control plane).
- **Every phase must keep CI green** before moving to the next phase:
  - `cd meg-rs && cargo test --locked --workspace`
  - `cd meg-rs && cargo build --locked --release --workspace`
  - `cd meg-plane && bun test --timeout 20000`
  - `cd meg-plane && bun build src/index.ts --target=bun --outfile=dist/plane.js`
  - `./meg-rs/target/release/meg.exe --version`
- **Lockfile rule**: Any `Cargo.toml` dependency change must update and commit `meg-rs/Cargo.lock` so `--locked` passes in CI.
- **No silent regressions**: Every bug fix and feature ships with targeted unit/integration tests.

---

## Phase 1 — Correctness, UTF-8 Safety & Hygiene
Goal: Eliminate crash/panic bugs, UTF-8 corruption, broken recursive globbing, and Windows CRLF edit failures.

- [x] **1.1 Fix UTF-8 FFI truncation panic** (`meg-rs/crates/meg-ffi/src/lib.rs`)
  - Use `floor_char_boundary(cap)` in `meg_read_file` so slicing never panics across `extern "C"`.
  - Add multi-byte UTF-8 boundary regression test.
- [x] **1.2 Fix UTF-8 truncation panic in observation trimmer** (`meg-rs/crates/meg-core/src/truncate.rs`)
  - Snap head slice and final `out.truncate(limit)` to valid UTF-8 char boundaries.
  - Add Unicode/multi-byte truncation regression test.
- [x] **1.3 Fix UTF-8 mojibake & regex recompilation in JSON repair** (`meg-rs/crates/meg-core/src/json_repair.rs`)
  - Iterate `raw.chars()` instead of raw bytes (`b as char`) in `repair_json` so non-ASCII text survives intact.
  - Cache compiled regexes with `std::sync::OnceLock` in `strip_think`, `recover_tool_call_markup`, and `recover_fenced_json`.
  - Add non-ASCII JSON repair regression test.
- [x] **1.4 Fix recursive `**` globbing** (`meg-rs/crates/meg-tools/src/fs_tools.rs`)
  - Rewrite `globset_lite` so `**/` matches zero or more directory segments (`a/b/c.rs` and root-level files both match `**/*.rs`).
  - Add shallow, deep-nested, and root-level glob tests.
- [x] **1.5 Windows CRLF-safe `edit_file` + compact diff output** (`meg-rs/crates/meg-tools/src/edit.rs`)
  - Try exact byte match first; if not found and file uses `\r\n`, retry normalized LF match while preserving original `\r\n` endings on write.
  - Include concise before/after line summary in tool output.
  - Add CRLF preservation and uniqueness tests.
- [x] **1.6 Fix `daemon_alive_probe`** (`meg-rs/crates/meg-core/src/finish_hook.rs`)
  - Check all `DAEMON_PROC_PATTERNS` (not only `.first()`).
  - Add unit test covering pattern matching helper.
- [x] **1.7 Drop unused `winapi` dependency** (`meg-rs/crates/meg-tools/Cargo.toml`, `meg-rs/Cargo.lock`)
  - Remove unused `winapi` crate (all Windows Job Object code uses `windows-sys`) and refresh `Cargo.lock`.
- [x] **1.8 Phase 1 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 2 — SOTA Coding Toolset & Multi-Step Undo
Goal: Give the agent fast, surgical workspace inspection and safe multi-step rollback.

- [ ] **2.1 Add `grep_files` content search tool** (`meg-rs/crates/meg-tools/src/grep.rs`, `lib.rs`, `registry.rs`)
  - Regex content search across workspace (skipping ignored dirs & binary/oversized files), optional `glob` filter, `max_results`, `context_lines`, cancellable via `AtomicBool`, returns `path:line:text`.
- [ ] **2.2 Add `list_dir` tool** (`meg-rs/crates/meg-tools/src/fs_tools.rs`, `registry.rs`)
  - Bounded directory listing with entry type (`dir`/`file`), byte sizes, and ignored-dir filtering.
- [ ] **2.3 Upgrade `read_file` with line numbers & `offset`/`limit`** (`meg-rs/crates/meg-tools/src/fs_tools.rs`)
  - Support optional 1-based `offset` and `limit` parameters with `N | line` formatting (preserving backward compatibility when omitted or needed).
- [ ] **2.4 Multi-step undo journal stack** (`meg-rs/crates/meg-tools/src/journal.rs`)
  - Replace single-slot `undo.json` with a bounded append/pop stack (e.g., last 20 edits) while keeping the strict "refuse if manually modified after MEG's edit" safety check.
- [ ] **2.5 Expand FFI bridge exports** (`meg-rs/crates/meg-ffi/src/lib.rs`, `meg-plane/src/ffi-bridge.ts`)
  - Expose `meg_grep_files` and bind both `meg_index_workspace` and `meg_grep_files` in `ffi-bridge.ts`.
  - Add Bun FFI tests for both.
- [ ] **2.6 Phase 2 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 3 — Native Tool Calling, Prompt Caching & Network Resilience
Goal: Upgrade model protocol from prompt-hacked JSON to native structured tool calling with prompt caching and retry backoff.

- [ ] **3.1 Tool JSON Schema definitions** (`meg-rs/crates/meg-tools/src/registry.rs`, `meg-core/src/lib.rs`)
  - Attach structured JSON Schema definitions alongside each tool in `ToolRegistry` (plus MCP/builtin schemas).
- [ ] **3.2 Native tool-calling payloads in `ShimClient`** (`meg-rs/crates/meg-core/src/shim_client.rs`)
  - Send OpenAI `tools` (`type: "function"`) and Anthropic `tools` (`input_schema`) when enabled, preserving `tool_call_id` / `tool_use_id` in message history.
  - Keep automatic text-JSON protocol fallback for local/smaller models or endpoints that reject `tools`.
- [ ] **3.3 Anthropic prompt caching (`cache_control`)** (`meg-rs/crates/meg-core/src/shim_client.rs`)
  - Emit structured `system` blocks and last-turn breakpoint with `cache_control: {"type": "ephemeral"}` on Anthropic requests so cached token savings actually activate.
- [ ] **3.4 Retry & backoff on `429` / transient `5xx`** (`meg-rs/crates/meg-core/src/shim_client.rs`)
  - Respect `Retry-After` header (capped), exponential backoff with jitter, and immediate cancellation via `CancellationToken`.
  - Add `wiremock` regression tests for native tool schemas, Anthropic `cache_control`, and `429` retry + cancellation.
- [ ] **3.5 Phase 3 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 4 — Wire Unwired Harness Guards & Upgrade Agent Loop
Goal: Connect existing built-in harness modules (`indexer`, `truncate`, `finish_hook`, `DeliverableGuard`) into the live loop and harden long-horizon execution.

- [ ] **4.1 Auto-inject Turn 1 `<workspace_map>`** (`meg-rs/crates/meg-core/src/lib.rs`, `meg-cli/src/main.rs`)
  - Pass workspace root cleanly into the loop (instead of relying on unset `MEG_INDEX_DIR`) so Turn 1 always starts with repo recon.
- [ ] **4.2 Enforce `truncate_observation` on all tool outputs** (`meg-rs/crates/meg-core/src/lib.rs`)
  - Bound every single tool, batch, MCP, skill, and delegate observation while preserving critical error lines.
- [ ] **4.3 Wire `DeliverableGuard` & `finish_hook`** (`meg-rs/crates/meg-core/src/lib.rs`, `meg-cli/src/main.rs`)
  - Guard verified deliverables against accidental overwrite/destructive bash and run conditioned `evaluate_finish` before accepting `Action::Final`.
- [ ] **4.4 Concurrent read-only `Action::Batch` execution** (`meg-rs/crates/meg-core/src/lib.rs`)
  - Execute read-only tools (`read_file`, `search_files`, `grep_files`, `list_dir`, `read_skill`) in parallel inside a batch; keep mutating tools (`write_file`, `edit_file`, `bash`) strictly sequential.
- [ ] **4.5 Doom-loop detection & auto-compaction** (`meg-rs/crates/meg-core/src/lib.rs`, `meg-cli/src/main.rs`)
  - Detect repeated identical failing tool calls and inject a corrective nudge instead of burning turns.
  - Add configurable `max_steps` (default 40 instead of hardcoded 20) and automatic mid-run transcript trimming/compaction when message char budget exceeds threshold.
- [ ] **4.6 Phase 4 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 5 — Control Plane & Delegation Hardening
Goal: Make Bun control plane state durable and Windows worker lifecycle leak-free.

- [ ] **5.1 Persistent `SessionTree` storage** (`meg-plane/src/session-tree.ts`, `index.ts`)
  - Persist session tree nodes/branches atomically under `.meg/sessions/` and reload on resume/fork.
- [ ] **5.2 Windows process-tree cleanup in `AgentManager`** (`meg-plane/src/agents.ts`)
  - Ensure worker timeout/abort kills the full child process tree on Windows (`taskkill /PID <pid> /T /F`) so no orphan workers survive.
- [ ] **5.3 Structured delegate progress & router sync** (`meg-plane/src/agents.ts`, `router.ts`, `index.ts`)
  - Emit structured worker turn/elapsed updates and allow `ModelRouter` to sync configured profiles without inventing fake cloud availability.
- [ ] **5.4 Phase 5 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 6 — End-to-End Offline Harness Benchmark & Verification Suite
Goal: Lock in measurable reliability gains with deterministic offline tests.

- [ ] **6.1 Multi-turn offline harness test suite** (`meg-rs/crates/meg-cli/tests/`, `meg-rs/crates/meg-core/tests/`)
  - Wiremock-driven E2E tests covering: Turn 1 workspace map, `grep_files` -> `read_file` -> CRLF `edit_file`, native tool calling + fallback, `429` retry, doom-loop recovery, parallel read-only batching, `finish_hook` recovery, and multi-step `/undo`.
- [ ] **6.2 Bun control-plane integration tests** (`meg-plane/`)
  - Verify `meg_grep_files` & `meg_index_workspace` via `bun:ffi`, persistent `SessionTree` fork/reload, and worker cancellation cleanup.
- [ ] **6.3 Phase 6 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 7 — CLI & Documentation Polish
Goal: Improve headless usability, streaming preview accuracy, and operator documentation.

- [ ] **7.1 Fix streaming `visible_preview` escape handling** (`meg-rs/crates/meg-cli/src/main.rs`)
  - Handle trailing incomplete backslash escapes during incremental JSON preview decoding.
- [ ] **7.2 Headless CLI flags** (`meg-rs/crates/meg-cli/src/main.rs`)
  - Add `--max-steps <N>` and `--json` output mode to `meg run` for scripting and evaluation harnesses.
- [ ] **7.3 Update `docs/native-harness.md`** (`docs/native-harness.md`)
  - Document new tools (`grep_files`, `list_dir`, paginated `read_file`), multi-step `/undo`, native tool calling, and new CLI flags.
- [ ] **7.4 Phase 7 Gate**: Full Rust + Bun tests and release build green; commit & push.

---

## Phase 8 — `meg-tui` Integration (Unfreeze UI)
Goal: Surface all core upgrades cleanly in the interactive Workstation TUI.

- [ ] **8.1 Surface diff previews, cache-hit tokens, auto-compaction notices, and multi-step undo feedback in `meg-tui`.**
- [ ] **8.2 Run full `meg-tui` adversarial/E2E suite + full workspace release build & CI verification.**