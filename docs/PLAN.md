# MEG Living Master Plan (v1.2 -- Micro-Tasked for Single-Builder Agent)
> Repository: `god88844-hub/MEG`
> Stack: `meg-rs/` (Rust Ring-0 Kernel) + `meg-plane/` (Bun/TS Control Plane) + `meg-eval/` (Python 3.11+ Offline Evals)
> Freeze Rule: `meg-rs/crates/meg-tui/` is FROZEN until Phase 8.
> Testing Rule: Run ONLY the Tier-1 Micro-Check per TODO (2-10s). Run Tier-2 full suite ONLY at Phase Exit.

---

## PHASE 0 -- GitHub Planning Lock & Repo Handshake
**Targets:** `AGENTS.md`, `CLAUDE.md`, `docs/`, `scripts/`, `.github/workflows/ci.yml`

- [x] **P0-01** Commit `AGENTS.md`, `CLAUDE.md`, `docs/ARCHITECTURE.md`, `docs/PLAN.md`, and `docs/STATE.md`.
  - *Micro-Check:* `git status` clean; files readable at repo root and `docs/`.
- [x] **P0-02** Create `scripts/init.ps1` (Windows 10 PowerShell 5.1) + `scripts/init.sh` (POSIX) verifying `cargo`, `bun`, `git`, `python`, and clean working tree.
  - *Micro-Check:* Run `powershell -File scripts/init.ps1`.
- [x] **P0-03** Create `scripts/check.ps1` + `scripts/check.sh` (Phase Gate runner) and `scripts/selftest.ps1` + `scripts/selftest.sh` (plants synthetic bug `delta`, proves `check` fails non-zero, reverts `delta`).
  - *Micro-Check:* Run `powershell -File scripts/selftest.ps1`.
- [x] **P0-04** Update `.github/workflows/ci.yml` to run `cargo test --workspace`, `bun test`, and a protected-path guard blocking unauthorized deletions in `tests/`. Export a concise 1-page inventory of existing `meg-rs` and `meg-plane` public APIs into `docs/API_INDEX.md` so Phase 1-5 code fits seamlessly.
  - *Micro-Check:* Validate YAML syntax and confirm `docs/API_INDEX.md` lists existing exports.

**Phase 0 Gate:** CI green; `docs/STATE.md` points to `P1-01`.

---

## PHASE 1 -- Rust Ring-0 Native Kernel, Tools, Sandbox & Security (`meg-rs/`)
**Language:** Rust | **Note:** Do NOT touch `meg-rs/crates/meg-tui/`.

- [x] **P1-01 Core 6-Tool Registry (`meg-tools/src/registry.rs`):** Lock default active tool menu to 6 primitives (`bash`, `read_file`, `write_file`, `edit_file`, `glob_grep`, `finish`) with strict JSON schemas, deadlines, byte caps, and `Copy-Constrained` argument span validation (file paths & commit SHAs checked against context spans).
  - *Micro-Check:* `cargo test -p meg-tools registry`
- [x] **P1-02 Hashline Reader & Atomic Patcher (`meg-tools/src/edit.rs`):** Tag every read line with 3-hex BLAKE3/FNV content hash (`42#f9a`), reject stale/mismatched edits before touching disk, support fallback unified-diff/search-replace, and preserve UTF-8/UTF-16 BOM and CRLF/LF endings.
  - *Micro-Check:* `cargo test -p meg-tools edit`
- [x] **P1-03 SWE-Agent ACI & Repeated-Read Suppression (`meg-tools/src/fs_tools.rs`):** Enforce 100-line bounded file viewer, 50-hit summarized search cap, and repeated unchanged-read suppression returning a short pointer when a file hash hasn't changed.
  - *Micro-Check:* `cargo test -p meg-tools fs_tools`
- [x] **P1-04 AST PageRank RepoMap & Important Files (`meg-core/src/indexer.rs`):** Add Tree-Sitter symbol extractor capped at ~1,000 tokens + `Aider` `ROOT_IMPORTANT_FILES` priority list + downstream caller lookup (prevents deleting imported symbols).
  - *Micro-Check:* `cargo test -p meg-core indexer`
- [x] **P1-05 RTK Log Deduplicator & Scratch Spill (`meg-core/src/truncate.rs`):** Implement `_compact_text` folding >2 repeated log lines (`... [previous line repeated N more times]`), spilling outputs >150 lines / >4,000 chars to `.meg/scratch/cmd_<id>.log` (`head=2400, tail=900`), and auto-bypassing truncation once `first_completion_claimed == true`.
  - *Micro-Check:* `cargo test -p meg-core truncate`
- [x] **P1-06 Out-of-Tree `side-git` Snapshots & Keep-Rate Tracker (`meg-tools/src/journal.rs`):** Maintain bare shadow git index in `~/.meg/snapshots/<repo-hash>/` for 1-ms pre-edit snapshots and `/undo` without polluting `.git`; log local `Keep Rate` when user reverts or immediately overwrites agent lines.
  - *Micro-Check:* `cargo test -p meg-tools journal`
- [x] **P1-07 CoW Worktree Manager (`meg-tools/src/worktree.rs`):** Create isolated worktrees (`~/.meg/worktrees/`) with 4-state lease log (`requested -> prepared -> lease_active -> scope_activated`, `cleanup: remove_if_clean`), linking `.env` and local caches (`node_modules`, `.venv`, `target/`) with zero reinstall wait.
  - *Micro-Check:* `cargo test -p meg-tools worktree`
- [x] **P1-08 Windows PowerShell 5.1 Adapter & Cross-OS Sandbox (`meg-tools/src/bash.rs`):**
  - Windows: Job Objects + UTF-8 console prefix (`[Console]::OutputEncoding = [System.Text.Encoding]::UTF8`) + POSIX idiom translator (`export` -> `$env:`, `rm -rf` -> `Remove-Item`, safe `;` chaining for PS 5.1).
  - Linux: `bubblewrap --unshare-all --unshare-net` (zero loopback control-plane exposure); macOS: `Seatbelt`.
  - Add cgroup/host RAM `make -j` cap (`max(1, RAM_GB / 2)`) and detached daemon FD handling (`</dev/null >/tmp/daemon.log 2>&1 &`).
  - *Micro-Check:* `cargo test -p meg-tools bash`
- [x] **P1-09 Stage-by-Stage Compound Bash Gate & Secret Airlock (`meg-tools/src/security_guards.rs`):**
  - Split `&&`, `||`, `;`, `|`, `$()`, backticks; if ANY stage is denied, abort entire command before Stage 1 runs.
  - Auto-lock `.env*` in `.gitignore`, block direct `.env` reads/writes, run Shannon-entropy + token-prefix (`sk-`, `ghp_`, `AIza`) pre-commit scanner, and scrub observations into `<SURROGATE_...>` tokens (`SAL` membrane; `disable_codebase_upload = true`).
  - Implement `ChildAuthority::clamp` bitmask intersection (`Child = Parent & Requested`).
  - *Micro-Check:* `cargo test -p meg-tools security_guards`
- [x] **P1-10 Competitor Bug Guards & Sealed-File Wall (`meg-tools/src/deliverable_guard.rs`):**
  - `AHE` Publish-State Shell Guard: parse passing check commands and hard-block `rm`/`git reset`/`git clean` on non-`/tmp` deliverables.
  - `gh` target-repo privacy pin (block external `--repo` leaks & autonomous `gh pr merge`, `git push --force`, `git tag`, `npm publish`).
  - Ring-0 `read_only_lock` flipped outside the LLM on `Esc` or stop phrases.
  - SHA-256 sealed-file hash manifest (`scalar-loop`) + post-run `git checkout base -- PROTECTED` wall (`tests/`, `migrations/`, `scripts/check.*`).
  - Lauren Tan `.hot.*` zero-allocation / zero-unseeded-random 60-FPS frame linter.
  - *Micro-Check:* `cargo test -p meg-tools deliverable_guard`
- [x] **P1-11 AEPN Petri-Net Supervisor & Undo-Price Gate (`meg-core/src/finish_hook.rs`):**
  - Implement P-invariant Petri-net transitions (`p_contract_locked`, `p_selftest_red`, `p_tests_green`, `p_critic_approved`, `p_human_approval_token`, `p_turn_budget`); editing code resets `tests_green` & `critic_approved` to `0`.
  - Classify actions by Undo-Price (`ONE_CLICK_UNDO` | `CLEANUP_UNDO` | `NO_UNDO`) + Timeout Destination Pre-Check before retrying external writes.
  - Add `atif-scan` single-model / single-effort lock when `--pinned-model` is active.
  - *Micro-Check:* `cargo test -p meg-core finish_hook`
- [x] **P1-12 Event Ledger, Signed Receipt & FFI Bridge (`meg-tools/src/ledger.rs`, `meg-ffi/src/lib.rs`):**
  - Append-only event stream (`session.vN.jsonl.zstd` / SQLite) logging failed generations off-history as `assistant/attempt`; OpenTelemetry (`OTel`) span exporter; tree-hash bound `.meg/receipt.json` verifier.
  - Export clean FFI functions to `meg-plane/src/ffi-bridge.ts` + idle-window GC/indexing hook during LLM network waits.
  - *Micro-Check:* `cargo test -p meg-ffi`

**Phase 1 Gate (Tier 2):** `cargo test --workspace` + `cargo clippy --workspace` 100% green.

---

## PHASE 2 -- TypeScript Control Plane, Multi-Model Routing & 5-Move Context (`meg-plane/`)
**Language:** TypeScript on Bun (`meg-plane/src/`)

- [x] **P2-01 Canonical Schemas & Contracts (`src/types.ts`):** Define TypeScript interfaces for `PreFlightTaskContract`, monotonic `MonotonicFeatureItem` (`features.json`), `ExecutionReceipt`, 4-Card Cookbook schemas (`Planner`, `Builder`, `Reviewer`, `Kickoff`), 7-field Swarm Handoff, `EDGNode`, `SubstitutionCertificate`, 5-enum `ToolErrorCategory` (`Transient | InvalidArguments | UnexpectedEnvironment | ProviderError | Timeout | UserAborted`), and `GOAL.md`.
  - *Micro-Check:* `bun test test/types.test.ts`
- [x] **P2-02 Deterministic Task State Machine & 3-Zone Workspace (`src/index.ts`, `src/session-tree.ts`):**
  - Implement state transitions `CREATED -> SCOPED -> PLANNED -> EXECUTING -> VERIFYING -> {VERIFIED | NEEDS_WORK | BLOCKED | FAILED | CANCELLED}` (only machine-verified receipt commits `VERIFIED`).
  - Initialize 3-zone workspace (`sources/`, `drafts/`, `published/`) + 6-field `progress.md` (`Task, Outputs, Completed, Decisions, Open issues, Next action`) + crash/restart replay from event ledger.
  - *Micro-Check:* `bun test test/plane.test.ts`
- [x] **P2-03 Turn-0 Bootstrap (<50ms) & Ambiguity Brake (`src/bootstrap.ts`):**
  - Probe OS/shell, git state, compilers, RAM, and DB/API schema pre-checks before Turn 1.
  - `Kimi K3` Turn-0 Ambiguity Gate: pause for `<=2` concise MCQ questions when high-impact ambiguities exist; write minor reversible assumptions to non-blocking `assumptions.md`.
  - *Micro-Check:* `bun test test/bootstrap.test.ts`
- [x] **P2-04 Universal Model Adapter & Governance Locks (`src/router.ts`, `meg-core/src/providers.rs`):**
  - Normalize OpenAI/Codex, Anthropic, Google, OpenRouter, and local Ollama/vLLM/OpenAI-compatible endpoints behind one `ModelClient` with capability probing (`tools`, `vision`, `context`, `reasoning_levels`, `ZDR noLog`).
  - Enforce `disabledProviders` *after* `--model` pin resolution (fixes `oh-my-pi #13079`) + `--pinned-model` single-model lock.
  - Provider-aware Thinking Preservation (keep thinking tokens within same model family thread; strip on cross-family handoff) + `verified` reasoning directive for local/mid models.
  - *Micro-Check:* `bun test test/router.test.ts`
- [x] **P2-05 5-Profile Budget Classifier, 3-Gear Thickness & Fusion (`src/fusion.ts`):**
  - Implement `HARNESSPRO` 5-profile classifier (`compact | lean [upgrade to standard if conf<0.9] | standard | deep | research`) with exact Lite/Medium/Heavy tool budgets (`econ / strict / final / rem`).
  - 3-Gear Adaptive Thickness: Gear 1 (4B-14B local: pre-action ledger + structured tools + `SELT`), Gear 2 (20B-35B open: `verified` + handshakes), Gear 3 (Frontier: thin bash-first freedom + `Reuse Base`).
  - Dual-Cache Model Fusion (Head thread + Sidekick thread with 240s keep-alive heartbeat) + `AgentBaton` 3-criterion readiness handoff (logfile transfer for hidden-CoT models like GPT-Sol; in-context transfer for Opus/Kimi/Gemini).
  - EPAM Circuit Breaker: halt/compact if single turn burns `>8,000` reasoning tokens with 0 tool calls or `>100:1` input:output ratio without test progress.
  - *Micro-Check:* `bun test test/fusion.test.ts`
- [x] **P2-06 5-Move `CONTEXT.md` Compiler & Short-Context `SSSM` (`src/context.ts`):**
  - `Cut` (`<=15` top rules, `<=6` tools, `<=2` examples, `AGENTS.md <= 60-100` lines, `MEMORY.md <= 2200` chars, `USER.md <= 1375` chars).
  - `Order` (byte-sorted static prefix stripping `<!-- -->` comments for `>90%` KV cache; `todo.md` + `PIN` recency override + `GOAL` at very bottom + every-4-turn Tail Goal Anchor).
  - `Fetch` (newest-first `mtime` sort, `<=3` hits); `Compress` (`EoCC`/`SAC` 5-part handoff briefs keeping current-attempt errors visible, stripping raw tracebacks across fresh attempts, `<40%` active context cap); `Split` (`>20`-page / `40K`-char reads -> 5-line reader subagent).
  - Single-pass media compaction: replace processed images/video frames with `@media:` disk pointers (fixes 39.5 MB replay bug).
  - *Micro-Check:* `bun test test/context.test.ts`
- [x] **P2-07 Code Mode, `HarnessMM` & 4-Stage `PreToolUse` Hooks (`src/extensions.ts`):**
  - Lazy `@tools/*` typed imports (`Code Mode`) + `HarnessMM` `EDG`/`GDR`/`CCS` cached decision-graph bypass gated by Substitution Paradox (`eliminated_reasoning > authorship + selection + recovery`).
  - `HARNESSPRO` 4-stage hook (`Economy -> Convergence -> Finalization Grace Window [max(8, rem)] -> Hard Stop Deny`) using absolute `$MEG_HOME` paths + `AHE` 7-risk `AfterToolHook` promoting warnings to top-of-turn `[FRAMEWORK REMINDER]`.
  - *Micro-Check:* `bun test test/extensions.test.ts`
- [x] **P2-08 Magnitude Router (`M0`-`M5`), `pstack` `/mode` & 7 Slash Commands (`src/commands.ts`, `meg-cli`):**
  - `M0` Nano (1 file, `<=3` turns, `$0.05`) through `M5` Mega-Engine (`250` turns).
  - `pstack` `/mode` (`feature | refactor | bug-hunt | migration`) with `>10`-file blast-radius pause.
  - 7 slash commands (`/verify`, `/commit` [session-edited files only], `/review`, `/refactor` [`jscpd`+`knip`+`ast-grep`], `/spec`, `/garden`, `/clear`) + `.meg/backlog.md` ticket drop (prevents scope poisoning).
  - *Micro-Check:* `bun test test/commands.test.ts`

**Phase 2 Gate (Tier 2):** `bun test` 100% green; prefix hash stability & crash-resume verified.

---

## PHASE 3 -- Memory, 8-Layer Skills & `CLI > MCP` Gateway (`meg-plane/`)
**Language:** TypeScript on Bun (`meg-plane/src/`, `meg-plane/skills/`)

- [x] **P3-01 4-Part Memory Stack & Bi-Temporal Ledger (`src/memory.ts`):**
  - Authoritative event ledger + lexical/AST/`ripgrep` search first (no mandatory code vector DB) + `STAIR` ToC leaf retriever for long docs.
  - `Reglos` bi-temporal records (`valid_time` + `learning_time`, closing intervals on updates rather than overwriting), 5 epistemic states (`KNOWS | BELIEVES | TRIED | VERIFIED | UNESTABLISHED`), SHA-256 source-file hash staleness invalidation, and `HYRA 2.0` 3-tier `Books` (`Observation -> Verified -> Convention [human approved]`).
  - Grok Bot Fast-Fact rule: block caching volatile live state (`price`, `plan_tier`, `ci_status`, `assignee`, `balance`).
  - *Micro-Check:* `bun test test/memory.test.ts`
- [x] **P3-02 Model-Tier Steering, `ACE` Librarian & Offline Dreaming (`src/dreaming.ts`):**
  - Index-separated rule workspace (`c_0 + compact index`).
  - Gear-aware memory steering: open salience on Gear 3 frontier models; mandatory `Read-Memory + Test-Theory` pre-hook on Gear 1-2 local/mid models.
  - Stanford 4-Agent `ACE` append-only `LEARNINGS.md` Librarian (`<=30` lines promoted to standing rules) + `[INSURANCE]` vs `[OPTIMIZATION]` rule gardening + Offline Dreaming pass (cross-checks notes against raw transcripts & builds linked `wiki/`).
  - *Micro-Check:* `bun test test/dreaming.test.ts`
- [x] **P3-03 Multi-Agent `Omnigraph` Git-Model Memory (`src/omnigraph.ts`):**
  - Per-agent memory branches keyed by `@key` slugs, record-level `merge_base` conflict detection (`409 divergent_update` rejection on stale full-record writes), `--snapshot` time-travel reads, and reviewer-only merge policy (`HTTP 403` for worker merges to `main`).
  - *Micro-Check:* `bun test test/omnigraph.test.ts`
- [x] **P3-04 8-Layer `SKILL.md` Loader & `Skill++` Index (`src/skills.ts`):**
  - 3-level progressive disclosure (`Metadata ~100 tok` -> `Body <500 lines` -> `references/ assets/ scripts/`), capping active load at `<=3` skills (`<=1` universal + `<=2` specific, `<=1,600` tokens, `>0.80` similarity dedupe) and `<=6 -> 4` tools post-refresh.
  - `Skill++` flat contrastive index (`Supports | Differs | Excludes`), path-scoped `.meg/rules/*.md` (`paths:` YAML), USC duplicate-skill merger, and `Data Forge` `manifest.yaml` multi-client packager (`--check` in CI).
  - *Micro-Check:* `bun test test/skills.test.ts`
- [x] **P3-05 `DDIPE` Skill Supply-Chain Scanner (`src/security-router.ts`):**
  - Scan all third-party `SKILL.md` files and embedded code blocks for implicit payload execution (`pip.conf`, `extra-index-url`, `requests.post`, `curl -d`, `authorized_keys`, `.ssh/`) before loading.
  - *Micro-Check:* `bun test test/security-router.test.ts`
- [x] **P3-06 `CLI > MCP` Enforcer & Hardened MCP Host (`src/mcp-host.ts`):**
  - Route `gh`, `supabase`, `stripe`, `vercel`, `git`, `curl` to native CLIs; restrict default MCP servers to `chrome-devtools`, `Context7`, and `sentry`.
  - Non-blocking async startup, per-caller timeouts, verdict-vs-transport error split, on-demand `uvx` ephemeral MCP spawning, zero turn-context exposure, scoped per-action tokens (no token passthrough), and Parallel/browser web-research connector.
  - *Micro-Check:* `bun test test/mcp-host.test.ts`

**Phase 3 Gate (Tier 2):** `bun test` 100% green; `DDIPE` catches 100% of poisoned skill fixtures; `Omnigraph` blocks lost updates.

---

## PHASE 4 -- Zero-Trust Verification Gauntlet & Loop Engineering (`meg-plane/` + `meg-rs/`)
**Language:** TypeScript + Rust

- [x] **P4-01 Verified Work Loop (`Loop 2`) & Stall/EFC Guards (`src/verifier_loop.ts`):**
  - Pre-run `check_cannot_fail` gate (`Green -> Red -> Green`), `<3,000`-char `FAILED / EXPECTED / SAW / file:line` feedback, path+digit-masked fingerprint `phi(e)`, `.loop/state.json` `tried` ledger, 3-pattern loop detector (`Repetition`, `Stagnation`, `A->B->A Cycling`), `EFC` (`I*V*R*M`) monitor, `MAX_SAME = 2` stall exit parking on `blocked/<id>`, and Karpathy Clean-Slate Replay (after `>4` messy retries, reset via `side-git` and re-implement cleanly in 1 fresh pass).
  - Concentric 4-Layer Diagnostic Router (`Harness -> Loop -> Context -> Prompt`), `DeclineCraft` calibrated abstention, and `Adaptive Replay Refinement`.
  - *Micro-Check:* `bun test test/verifier_loop.test.ts`
- [x] **P4-02 Patched `RISEHARNESS` 4-Stage Handshake (`src/handshake.ts`):**
  - Unified `+SE/+OO` (validates `verification_cmd` + `expected_exit_code` and runs ONCE in 30s out-of-band sandbox, failing open on infra error) + `spec_check_failure_ledger` (`pytest --last-failed` keyed by artifact SHA).
  - `+DI` (designated script invocation / unverified diff check) + `+IR` (reachable single-signature AND conflicting-readings branches), bounded at `MAX_WITHHOLDS = 1` per gate.
  - *Micro-Check:* `bun test test/handshake.test.ts`
- [x] **P4-03 Verification Engineering (`L0`-`L5`) & Maker-Blinded `CHECKS.md` (`src/checks_gate.ts`):**
  - Block completion below `L4 . Proof Attached`.
  - Keep `CHECKS.md` scoring blinded from Maker; return only failed lines (max 3 rounds); live ground-truth recomputation + `>30%` delta flagging + cross-tenant leak scan + `MiRL` 8-misbehavior gate + `CAAF` monotonic state locking (`read_only: true` on passing criteria).
  - *Micro-Check:* `bun test test/checks_gate.test.ts`
- [x] **P4-04 `L1` After-Edit Reminders & Cold Cross-Model `L3` Critic (`src/critic.ts`):**
  - Disable wasteful `L2` self-recheck prompts; emit `L1` after-edit reminders (capped at 4/run).
  - Harness assigns a different model family (`>=` writer capability) in a fresh context seeing ONLY Contract + `git diff` + test logs (`Claim-guided: changes`, `Width = 1..4` sequential critics before 1 revision, conclusion-blind mode, `/tmp` throwaway probes, `PROXY`/`SHARED MISTAKE` bans, non-mutation enforced).
  - *Micro-Check:* `bun test test/critic.test.ts`
- [x] **P4-05 `Poteto Pyramid` Linters, Mutation Gate & Statechart Checker (`src/poteto_checks.ts`):**
  - AST anti-stub/false-zero scanner (`TODO`, `501`, `catch { return [] }`, weakened assertions) + Remediation-Injecting Linter formatter (`[REMEDIATION FIX]: ...`).
  - `tesearch`-style fast mutation check rejecting hollow tests + BFS Statechart/`TLA+` reachability verifier (deadlock & forbidden-state checks) + optional 3-question Maintainer Comprehension Quiz on diffs `>300` lines.
  - *Micro-Check:* `bun test test/poteto_checks.test.ts`

**Phase 4 Gate (Tier 2):** Cheating-builder test suite (empty diff, edited test, mocked pass, false-zero catch, single-method OCR, repeat loop) blocked 100%.

---

## PHASE 5 -- Multi-Agent Graphs, Goal/Motion Engineering & GitHub/CI (`meg-plane/`)
**Language:** TypeScript on Bun (`meg-plane/src/`)

- [x] **P5-01 Algebraic Orchestration (`Maestro Order`) & Code-Routed DAG (`src/graphs.ts`):**
  - Single-agent default; Early-Start independence test required before fan-out.
  - 4 combinators (`Sequential`, `Parallel`, `Verify`, `Recurse`) + `Blackboard` (`logodds, calls, coverage, ABSTAIN`).
  - Default to streaming `pipeline()` over blocking `parallel()` barriers; plain code (`if/switch`) routes typed node contracts; LIFO Saga rollback stack + partitioned parent budgets.
  - *Micro-Check:* `bun test test/graphs.test.ts`
- [x] **P5-02 DeepSeek Router-Worker, 4-Shape Swarm & Return Edges (`src/agents.ts`):**
  - Persistent Bash-less Global Router (Trigger prefill batching + Thread Subscription direct pass + Full-Duplex `start()`/`stop()`).
  - 4-Shape classifier (`FAN-OUT <=10 items/worker`, `CHAIN`, `DEBATE` [independent proposals before interaction], `SUPERVISOR`) + 7-field handoff JSON + `TeamCreate` 1-File-1-Owner locks (`<=5` teammates, early shutdown).
  - **Unit-Only Return Edge** (freezes passed slices read-only; returns ONLY failed unit with file fence) + **Long Learning Edge** to Splitter + **`Planfence`** input-hash rebuilds + **Loop-Until-Dry `seen`** deduplication (`confirmed U rejected`).
  - *Micro-Check:* `bun test test/agents.test.ts`
- [x] **P5-03 7-Rule File Swarm (`src/file_swarm.ts`):**
  - Enforce `BOARD.md`, pairwise scope-tightened `tasks/T[n].md`, `outputs/T[n].md` requiring final-line `END` sentinel, `checks/CONFLICTS.md`, `[T#]` paragraph provenance in `FINAL.md`, and `<=30`-line `LESSONS.md`.
  - *Micro-Check:* `bun test test/file_swarm.test.ts`
- [x] **P5-04 Goal & Motion Engineering (`src/goals_motions.ts`):**
  - Dual-Cadence handoff (`STATE.md` loops -> `GOAL.md` + `goal-budget.md` + quiet `update_goal` telemetry + `/goal status|pause|resume|clear`).
  - Anti-Goal-Gaming floor invariants (`total_test_assertions >= baseline`, `0 protected files deleted`).
  - Background Entity Context Gatherers + horizontal **Motion Agents** grouping pending drafts into a Homogeneous Batch Review Queue for cross-entity tacit taste transfer.
  - Isolated scheduled routine runner with 0-token pre-run delta check (`routines/state/<name>.txt`) in `dontAsk` mode.
  - *Micro-Check:* `bun test test/goals_motions.test.ts`
- [x] **P5-05 GitHub IssueBot, CI Triage & PR Hygiene (`src/github.ts`):**
  - **IssueBot 5-Stage Graph:** Tool-less Planner reads untrusted issue -> code-validated contract -> raw issue deleted (Lethal Trifecta broken) -> worktree Builder -> parallel Verify (`test+lint+typecheck`) -> cold Reviewer -> idempotent Draft PR -> CI watch -> human merge.
  - **5-Category CI Triage (`env | flake | bug | dep | infra`)**, 4-tick PR watcher (`Watching 1/3..3/3 -> Ready`), conflict-only rebase, `Qodo` approval invalidation on new commits, multi-PR session summary, post-merge staging QA, and 2-PR missed-case rule extraction.
  - *Micro-Check:* `bun test test/github.test.ts`

**Phase 5 Gate (Tier 2):** Parallel worktree test (0 collisions), Unit-Only Return Edge test, Lethal Trifecta injection test, and Anti-Goal-Gaming test all green.

---

## PHASE 6 -- All 29 Software Domains, Multimodal Media & Daily Usage
**Language:** TypeScript Skills (`meg-plane/skills/`) + On-Demand Domain Oracles (`meg-eval/oracles/`)
**Rule:** Zero tools added to the default 6-tool base prompt; every pack loads lazily via `SKILL.md`.

- [ ] **P6-01 CLI Multimodal Media & Paste Ingestion (`meg-plane/src/media.ts`):**
  - Handle drag-and-drop & clipboard (`Ctrl+V`) images/videos, `ffmpeg` 1fps scene-change keyframe extraction, and deterministic OCR/layout fallback for text-only local models.
  - *Micro-Check:* `bun test test/media.test.ts`
- [ ] **P6-02 Pack 1 -- Vibe Coders, PM, Scrum & UI/UX Wireframing (Domains 1-3):**
  - Vibe-to-Spec front-end: vague prompt -> Elephant `PRD.md` -> 3 stateless Goldfish critics (Contrarian, Edge-Case, Token Economist) -> 1-feature slices in fresh contexts.
  - `AskUserQuestion` structured clarification forms + `/refactor-state-machines` (`UX/DX/AX`) + `/taste` anti-slop UI pack (`a11y`, `smell.md`, `typeset.md`, 1440px/360px responsive checks).
  - *Micro-Check:* `bun test test/pack_vibe.test.ts`
- [ ] **P6-03 Pack 2 -- Web, Full-Stack, Backend, Mobile, Desktop, DB, Enterprise & QA (Domains 4-15):**
  - Sub-50ms LSP diagnostics on edit + live DAP debugger (`debugpy`/`dlv`/`lldb`) + Playwright E2E live click-through + `Bombadil` property-based UI fuzzing + deployment bootstrap-ordering check.
  - Desktop GUI OS Accessibility Tree bridge (`agent-desktop` JSON tree with PII scrubbing).
  - Binary Office/Data skill (`office-doc-write` `.docx` + `s7->s30` `PercentCheckHarness` `.xlsx` `(0,1) -> *100`).
  - *Micro-Check:* `bun test test/pack_appdev.test.ts`
- [ ] **P6-04 Pack 3 -- 2D/3D Games, Physics & ARC-AGI-3 World Models (Domains 16-17):**
  - `M5` 7-Subsystem ECS template (`SCHEMA.md` + `.hot.*` zero-alloc 60-FPS lint) + WebGL 0-console-error check + automated BFS playtester.
  - `Kepler 1.0` / `Schema` world-model (`step(state, action)` 100% `run_backtest` certification + auto-`RESET` on `GAME_OVER`).
  - *Micro-Check:* `bun test test/pack_games.test.ts`
- [ ] **P6-05 Pack 4 -- Graphic Design, SVG, 60-FPS Motion Video & 3D/CAD (Domains 18-20):**
  - 2-Stage Parametric Skeleton rule (named joints/anchors before raw coordinates) + `BrickBench` DSG visual question graph.
  - `3DCodeVerse` 17-bundle skill library & 28-rule routing table (`bpy`, `CadQuery`, `OpenSCAD`, `Three.js`, `URDF`, `GLSL`, `OpenGL` + spatial collision/connectivity checks) + 18 `VisualBench` deliverable checklists.
  - Deterministic `window.seek(t)` 60-FPS Motion Studio (closed-form multi-target springs `track(t, keys)`, `librosa` beat sync, OpenCV `#00B140` chroma cutouts, 3 FFmpeg sheets `contact.png`, `strip.png`, `phone.png`, plus `2000ms`/`15000ms` frame-diff check).
  - *Micro-Check:* `pytest meg-eval/oracles/test_visual_motion.py`
- [ ] **P6-06 Pack 5 -- OS/Kernel, Compilers, Embedded/IoT, Edge, Robotics, Quantum, Web3, DevOps/Cloud/SRE, AppSec & Data/AI-ML (Domains 21-29):**
  - QEMU boot / GCC differential compiler oracle, `ChartEvo` `SELT` compiled local toolchains (`3%-5%` near-miss audit + uncertain-interval midpoint ban), read-only AppSec audit, `foundry`/`slither` (Web3), `Qiskit` (Quantum + Competing-Algorithm search), and `ROS2`/`URDF` (Robotics) templates.
  - *Micro-Check:* `pytest meg-eval/oracles/test_systems_selt.py`
- [ ] **P6-07 Pack 6 -- Daily Usage, Second Brain & Team Rooms:**
  - Flat-file Obsidian/Markdown Second Brain (`notes/`, `people/`, `projects/`, `inbox/`, `briefs/`, `decisions.md`) with Karpathy 6-Pass Wiki Compiler + `/remember`, `/recall`, `/resurface` (1/3/7/21/60d), `/review`, `/morning-brief`, `/weekly-review` (`dontAsk` draft-only).
  - Local `Alook`-style team rooms (`#intel` vs `#alerts`) + `Nimble`-style dated web snapshot diffing + voice keyterm support.
  - *Micro-Check:* `bun test test/pack_daily.test.ts`

**Phase 6 Gate (Tier 2):** All 6 pack smoke tests green; base prompt still strictly 6 core tools.

---

## PHASE 7 -- Offline Evaluation, Benchmarking & Controlled Evolution (`meg-eval/`)
**Language:** Python 3.11+ (`uv` / `pytest` / `Inspect AI`) -- **Strictly Offline**

- [ ] **P7-01 4-Layer Eval Runner (`meg-eval/runner.py`):** Layer 1 `<5s` behavioral trace assertions; Layer 2 security/adversarial (`DDIPE`, `InjecAgent`, `Blindfold` audit timing secrecy, `SABench` side effects); Layer 3 private 50-200 golden tasks; Layer 4 public adapters (`SWE-bench Verified/Pro`, `Terminal-Bench 2.0/2.1` in `--pinned-model` mode, `VisualBench`, `ARC-AGI-3`, `Strength-2` 18-config orthogonal stress grid).
  - *Micro-Check:* `pytest meg-eval/test_runner.py`
- [ ] **P7-02 Reliability & DORA Metrics (`meg-eval/metrics.py`):** Compute `Pass@1`, `Pass^4`, `Flip`, unbiased `Pass@k`, `CoP` (Cost per Verified Outcome), **Harness Constraint Effect**, **Defect Escape Rate**, **False Completion Rate**, and `MAD-RSI` `Pass^M` / `D_q`.
  - *Micro-Check:* `pytest meg-eval/test_metrics.py`
- [ ] **P7-03 Label-Free `TestSynth` & Trace Attribution (`meg-eval/testsynth.py`):** Generate 10 container-validated `unittest` cases per failure (banning `/empty_product` oracle scraping; serial `infra_error` repair) + `HarnessFix` `HTIR` fault attribution + `Shor` component priority ranker.
  - *Micro-Check:* `pytest meg-eval/test_testsynth.py`
- [ ] **P7-04 Offline Controlled Candidate Evolution (`meg-eval/evolve.py`):**
  - **Gated Modular Evolution (`arXiv:2611.22299`):** Mutate 1 module at a time behind held-out gates + token caps.
  - **`GUARD` Regret Control (`arXiv:2611.23476`):** Turn-0 checklists + directional regret + Soft-HURT trust-region gate (0.00 pp forgetting) + rejection ledger.
  - **`OO-HARNESS` + `HARNESSPRO` + `MAD-RSI` + `RuHarness` + `SkillAdam` + `EnvOpt`:** Cluster-only eval on `D_c` with priority decay & `add|merge|split`, 10-task DPP coreset (2 unanimous fail + 2 unanimous pass anchors), `Lite/Med/Heavy` Pareto selection, 3-config behavioral check, `fires_on_stable == False`, peak-tracking admission gate, and Dual-Rubric Governance Gauntlet (`CLEAN != GAMING`).
  - **Hard Rule:** Outputs ONLY an **unmerged GitHub PR** with before/after receipts for human sign-off. Zero auto-merge; zero live Ring-0 edits.
  - *Micro-Check:* `pytest meg-eval/test_evolve.py`
- [ ] **P7-05 Bitter-Lesson Model-Upgrade Decay Sweep (`meg-eval/ablate.py`):** On new model checkpoints, run 3-trial single-component ablations and open a PR to delete obsolete scaffolding.
  - *Micro-Check:* `pytest meg-eval/test_ablate.py`

**Phase 7 Gate (Tier 2 + Tier 3 sample):** Offline eval & candidate PR dry-run green with 0 `GAMING` verdicts and 0 regressions.

---

## PHASE 8 (FINAL PHASE) -- TUI Excellence Upgrade (`meg-rs/crates/meg-tui/`)
**Language:** Rust (`meg-rs/crates/meg-tui/`)

- [ ] **P8-01 CSI 2026 Differential Renderer & Titanium Theme (`theme.rs`, `viewport.rs`):** Synchronized cell diff updates (`\x1b[?2026h`..`l`), `titanium` dark/light palettes, `StatusLine` presets (`default|minimal|compact|full|nerd|ascii`), and `Symbol` presets (`unicode|nerd|ascii`).
  - *Micro-Check:* `cargo test -p meg-tui theme`
- [ ] **P8-02 Devin-Clean Collapsible `ToolCard` & `L0-L5` Badges (`stream.rs`, `chips.rs`):** `framedToolCard` / `plainToolCard` disclosures, inline `Hashline` unified diffs, and `L0 -> L5` verification badges.
  - *Micro-Check:* `cargo test -p meg-tui stream`
- [ ] **P8-03 OMP `JobsPanel`, `JobWidget` Tiles & `/jobs` `JobTree` (`job.rs`, `dashboard.rs`):** Live 1s ticking tiles (`task | bash | eval`), status dots (`running|completed|failed|cancelled`), and modal `/jobs` hierarchy sheet.
  - *Micro-Check:* `cargo test -p meg-tui job`
- [ ] **P8-04 OMP `Alt+A` 7-State `AgentHub` & `PinnedHUD` (`workspace.rs`, `runtime.rs`):** Isolated worktree roster (`running|completed|failed|cancelled|idle|parked|aborted`), `[S]` steer / `[K]` kill controls, 1-line `PinnedHUD` live tool peek, and Actively AI **Motion Batch Review** view.
  - *Micro-Check:* `cargo test -p meg-tui workspace`
- [ ] **P8-05 Rich Composer & Live Telemetry HUD (`composer.rs`, `picker.rs`, `input.rs`):** `@path` fuzzy picker, `>5`-line bracketed-paste pills, `AskUserQuestion` interactive modal, Kitty/iTerm2 inline image preview, `assumptions.md` badge, and live Self-Budget / KV-Cache % / EFC / `/goal` bar.
  - *Micro-Check:* `cargo test -p meg-tui composer`
- [ ] **P8-06 Full TUI Adversarial & E2E Pass (`tests/`):** Extend and run all existing `meg-tui` adversarial & E2E tests across Windows 10 ConPTY/PowerShell, macOS, and Linux.
  - *Micro-Check:* `cargo test -p meg-tui`

**Phase 8 Gate (Final Release):** Full workspace green (`cargo test --workspace` + `bun test` + `pytest`).