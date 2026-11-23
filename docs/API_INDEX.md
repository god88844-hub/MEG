# MEG Public API Index

Concise inventory of current public exports across `meg-rs/` and `meg-plane/` so Phase 1-5 additions extend existing contracts cleanly without breaking frozen `meg-tui/`.

> **Frozen Interface Note:** `meg-rs/crates/meg-tui/` directly constructs `meg_core::ChatMessage { role, content }` and reads `ToolRegistry.stubs`. Keep those field layouts stable until Phase 8.

---

## 1. `meg-rs/crates/meg-tools` (Ring-0 Tool Kernel)

| Module | Key Public Exports |
| :--- | :--- |
| `registry.rs` | `ToolResult`, `ToolSchema`, `ToolFn`, `ToolRegistry::{new, with_core_tools, with_cancellation, register, register_with_schema, tool_schemas, execute, names}` |
| `fs_tools.rs` | `resolve`, `read_file` (raw or paginated 1-based line numbers), `write_file`, `list_dir`, `search`, `search_cancellable` |
| `edit.rs` | `str_replace` (unique match, CRLF-preserving, `replace_all`) |
| `grep.rs` | `grep_files`, `grep_files_cancellable` |
| `bash.rs` | `run`, `run_cancellable` (Windows Job Objects + timeout + output cap) |
| `journal.rs` | `FileCheckpoint::{capture, commit}`, `depth`, `undo` (20-step stack in `.meg/tui/undo.json`) |
| `deliverable_guard.rs` | `DeliverableGuard::{from_goal, lock_verified_deliverable, unlock_deliverable, check_write, check_bash}`, `check_single_file_cleanliness`, `deep_artifact_sanity`, `whitelist_sweep_single_file` |

## 2. `meg-rs/crates/meg-core` (Agent Loop, Providers & Guards)

| Module | Key Public Exports |
| :--- | :--- |
| `lib.rs` | `Action`, `ChatMessage`, `LoopEngine`, `TurnOutcome`, `LoopConfig`, `OllamaConfig`, `OllamaClient`, `parse_action_value`, `parse_model_output`, `is_read_only_tool`, `run_loop`, `run_loop_with_history`, `run_loop_with_config`, `compact_transcript` |
| `shim_client.rs` | `ShimClient::{new, with, with_profile, native_tools_active, healthy, models, chat_stream, chat_stream_cancellable, chat}`, `ToolSpec`, `NativeToolCall`, `Delta`, `ROLE_ASSISTANT_TOOL_CALLS`, `ROLE_TOOL_RESULT`, `encode_native_tool_calls`, `encode_native_tool_result`, `extract_native_tool_calls`, `default_base_url`, `default_model` |
| `providers.rs` | `Secret`, `ProviderKind`, `ProviderProfile`, `ProviderSettings`, `ProviderStore::{global, load, save, upsert, activate, remove}`, `detect_key_provider`, `config_dir`, `credential_scope`, `api_key`, `has_key`, `vault::{get, set, remove, get_scoped, set_scoped, remove_scoped}` |
| `provider_catalog.rs` | `ProviderPreset`, `PROVIDER_PRESETS`, `provider_preset`, `key_preset`, `is_local` |
| `indexer.rs` | `index_workspace`, `index_workspace_with_budget` |
| `truncate.rs` | `truncate_observation` (UTF-8 safe head/tail + critical-line preservation) |
| `json_repair.rs` | `repair_json`, `extract_json_object`, `strip_think`, `recover_tool_call_markup`, `recover_fenced_json`, `parse_action` |
| `finish_hook.rs` | `FinishHookConfig`, `FinishVerdict`, `local_deliverable_check`, `polyglot_cleanliness_failures`, `scm_structural_ok`, `evaluate_finish`, `goal_wants_live_daemon`, `daemon_pgrep_pattern`, `daemon_alive_probe`, `GIT_NONINTERACTIVE_ENV` |
| `plane_client.rs` | `PlaneClient::{spawn, call, call_with_events}` |
| `cancel.rs` | `CancellationToken::{cancel, is_cancelled}` |
| `build.rs` | `is_build_goal`, `is_explicit_2d`, `mechanical_check`, `build_site` |

## 3. `meg-rs/crates/meg-ffi` (C-ABI `cdylib` -> Bun)

`extern "C"` exports (every returned `*mut c_char` must be freed via `meg_free`):
`meg_free`, `meg_search_files`, `meg_grep_files`, `meg_list_dir`, `meg_read_file`, `meg_edit_file`, `meg_truncate_observation`, `meg_parse_action_variant`, `meg_index_workspace`

## 4. `meg-plane/src/` (Bun/TypeScript Control Plane)

| File | Key Public Exports |
| :--- | :--- |
| `index.ts` | `handleRpc` (JSON-RPC 2.0 stdio dispatch: `runtime.*`, `router.*`, `session.*`, `agents.*`, `skills.*`, `plugins.*`, `mcp.*`) |
| `ffi-bridge.ts` | `FfiLib`, `loadFfi` |
| `router.ts` | `ProviderKind`, `ModelEntry`, `StoredProviderProfile`, `RouteRequest`, `RouteDecision`, `ModelRouter` |
| `session-tree.ts` | `SessionNode`, `SessionTree`, `validateSessionId`, `sessionsDir`, `sessionFilePath` |
| `agents.ts` | `AgentEvent`, `killProcessTree`, `AgentManager` |
| `extensions.ts` | `PromptResource`, `Plugin`, `configRoot`, `WorkspaceExtensions` |
| `mcp-host.ts` | `McpTool`, `McpConfig`, `McpHost` |
| `security-router.ts` | `isSecurityOrCryptoTask`, `RouteVerdict`, `routeTask` |