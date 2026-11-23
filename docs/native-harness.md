# Native MEG workstation

Run `meg` to choose a folder first. Any existing folder can be a workspace;
it does not need a Git repository or package manifest. When launched inside a
project, the browser starts at its parent with that project selected. Arrows
select, Enter opens, Right/Tab browses into a folder, and Left/empty Backspace
goes up. Ctrl+O opens the current directory. Ctrl+N creates a folder: type a
single name, then Enter explicitly creates and opens it. Typing alone never
creates a directory; duplicate names and invalid paths show an error.
Paste a path or use Alt+1/2 for recent folders; Ctrl+A clears the path.
Escape cancels creation or exits the startup screen. Project-root suggestions
are available with Ctrl+P; `MEG_PROJECTS_DIR` adds project containers.
`/workspace` switches workspace during a session after active work has stopped.

`meg -C "C:\projects\my project"` opens that workspace directly. `--no-picker`
uses the current directory; `--continue` resumes its latest session; `--resume
<UUID>` selects one. `-m <model>` and `--effort low|medium|high|max` override
saved defaults. `meg run "<goal>"` runs headlessly in the current directory with
the selected provider. `meg --help` lists CLI options.

Headless options precede the goal: `meg run -C "C:\projects\my project" -m <model>
--effort high "<goal>"`. `--headless` is accepted for compatibility. Use `meg run
-- "-m is literal prompt text"` to preserve a goal that looks like an option.
Missing arguments fail before a provider request. `MEG_OFFLINE=1` refuses headless
inference, and an exhausted turn budget returns a nonzero exit status.

## Local and cloud connections

MEG has no account, subscription or MEG login requirement. Cloud providers still
require their own credentials. `/provider` (also `/connect`) opens a searchable
service catalog. Search by provider name, select it, and paste into **API key**.
Enter connects. Ctrl+E edits a connection or opens advanced fields; Delete removes
the connection and its key. Ctrl+N adds a custom compatible endpoint.
The catalog includes OpenRouter, OpenAI, Anthropic, NVIDIA NIM, Google Gemini,
Groq, DeepSeek, Mistral, xAI, Fireworks, SiliconFlow and Nebius, plus local Ollama,
LM Studio, llama.cpp and vLLM endpoints. `/provider add nvidia` opens its key prompt;
`/provider add custom` opens the complete endpoint editor. Numeric shortcuts 1–5
remain compatible with OpenRouter, OpenAI, Anthropic, Ollama and LM Studio.
No personal gateway is enabled or contacted by default. Existing personal
profiles remain local to the operator's configuration.

The six fields are name, protocol, API base URL, optional default model, API key
and optional API-key environment-variable name. Tab/Shift+Tab changes fields,
Space changes protocol, Ctrl+A replaces a field, and Enter/Ctrl+S saves. Paste into
**API key** with the terminal's paste shortcut, usually Ctrl+V or Ctrl+Shift+V.
The key is masked and stored in Windows Credential Manager at
`MEG/provider/<profile-id>` for the normal configuration. Alternate `MEG_CONFIG_DIR`
roots have isolated credential namespaces, so test configurations cannot read,
replace or delete an operator's connections. Keys are excluded from public
settings, chat and session files.
An alternate configuration created by an older build may need its provider key
entered again. Legacy vault entries are retained; MEG does not copy the standard
configuration's credentials into another configuration directory.
Closing the editor discards an unsaved key. Empty key input while editing retains
the stored key; Delete on the profile removes it.

Pasting a distinctive prefix such as `sk-or-v1-`, `sk-ant-` or `nvapi-` into the
provider list selects its service and opens a masked prompt. An opaque pasted key
stays masked while you choose its issuer. Generic `sk-` keys are ambiguous: choose their provider
or API base URL. MEG never sprays credentials across candidate services. Explicit
custom URLs remain in place. Models are fetched after saving; cloud models are
chosen explicitly rather than silently selecting the first potentially paid model.
OpenRouter discovery uses its authenticated `/models/user` catalog.

Alternatively enter an existing environment-variable name such as `OPENAI_API_KEY`
in the last field. Its value is resolved from MEG's process environment. On other
platforms use this alternative; persistent vault storage is currently Windows-only.
Never put keys in the URL or a chat message.

Base URLs include their version path: `http://localhost:11434/v1` for Ollama,
`https://api.openai.com/v1`, or `https://api.anthropic.com/v1`. OpenAI-compatible
profiles accept compatible local/cloud gateways. Anthropic uses its native
Messages protocol, headers, SSE and usage. Gemini uses Google's documented
OpenAI-compatible endpoint. No key is required by unauthenticated local
endpoints. MEG does not install or start model servers.

`/provider use <name>`, `/provider test`, `/model refresh` and `/model <id>` are
command equivalents. `/model` combines models from connected services and running
local runtimes, with provider identity retained even for equal model names. Ctrl+R
refreshes, Ctrl+N connects another service, and search/selection survive background
refresh. Local discovery checks Ollama 11434, LM Studio 1234, llama.cpp 8080 and
vLLM 8000; custom server addresses can be configured. Ollama's native `/api/tags`
is a fallback for servers without a model-list compatibility route. A server's
advertised models are distinct from files merely downloaded somewhere on disk.
Missing servers, empty installations and cached/unavailable catalogs have explicit
states. Discovery is bounded and asynchronous, with four concurrent model checks.
Manual model IDs work
when `/models` is unavailable. `/model <provider-id>::<model-id>` switches both.
Switching retains the conversation; subsequent context goes to the new endpoint.
Tokens use provider-reported usage, with missing usage/pricing shown honestly.

Public configuration lives in `%APPDATA%\MEG`; `MEG_CONFIG_DIR` overrides it.
The explicit `MEG_TUI_BASE_URL`, `MEG_TUI_MODEL`, optional `MEG_TUI_API_KEY`
environment setup remains supported. Workspace sessions/preferences live in
`.meg/tui`. New stores create a private-state `.gitignore`. Session files contain
conversation/tool output and should stay private. `/settings` opens controls for
connections, models, appearance, effort, modes, workspace and extensions.
`/config` displays effective public settings.

## Commands and editing

Type `/` for the searchable menu; `/help` lists commands, including discovered
skill/plugin shortcuts. Enter sends; Alt+Enter/Ctrl+Enter inserts a newline.
Up/Down recalls drafts, PgUp/PgDn scrolls chat, Ctrl+T expands or folds thinking.
Thinking starts collapsed so streamed reasoning does not dominate the transcript.
A small eight-frame-per-second spinner runs while waiting or receiving reasoning,
including when the block is folded. It stops when the answer starts, the run
finishes, or cancellation completes. Clipboard
text becomes an editable draft. Ctrl+C closes a dialog, clears a draft, stops an
active turn, or exits when idle. `/stop` cancels root/delegated operations;
`/exit` shuts down owned processes.

Local commands include `/new`, `/sessions`, `/resume`, `/fork`, `/clear`,
`/compact`, `/export`, `/queue`, `/stats`, `/status`, `/context`, `/theme`,
`/effort`, `/todo`, `/tools`, `/settings` and `/config`. `/compact` archives older context and
retains excerpts/recent messages, without a semantic-summary claim. `/undo`
restores the last journaled file-tool mutation, not shell changes. `/plan <task>`
permits read-only tools; `/code [task]` selects Act. `/permissions readonly|act`
changes mode. Act permits writes and shell execution; this is not an OS sandbox.
`/init` asks the selected model to improve workspace AGENTS.md and uses a turn.

## Skills and plugins

Skills are actual SKILL.md bodies. Discovery reads bundled skills, configuration
root `skills/`, `~/.agents/skills`, then workspace `.claude/skills`, `.agents/skills`
and `.meg/skills`. A skill is a subdirectory containing SKILL.md, optionally with
YAML front matter `name` and `description`. Later equal names override earlier
ones. `MEG_USER_SKILLS_DIR` overrides the home directory. `/skills refresh`
reloads; `/skills` lists; `/skill:<name> <task>` applies a workflow. Models can
load a body with `read_skill`.

`/plugins add "<local-directory>"` installs a declarative plugin. The directory
needs `meg-plugin.json` or `.claude-plugin/plugin.json`, for example:

```json
{"name":"project-checks","skills":["skills"],"mcpServers":{}}
```

`commands/review.md` becomes `/project-checks.review`; `$ARGUMENTS` substitutes
prompt text. `/plugins enable|disable|remove <name>` manages the registry;
removal leaves source files intact. Plugin MCP names use `<plugin-name>_` prefixes.
Arbitrary JavaScript hooks, marketplaces and cross-harness plugin compatibility
are not implemented.

## MCP and workers

The Bun plane uses the official MCP SDK: stdio, Streamable HTTP and SSE, real
initialization/discovery, tool schemas, errors, cancellation and shutdown.
Configuration merges configuration-root `mcp.json`, workspace `.mcp.json`,
`mcp_config.json`, `.meg/mcp.json`, then enabled plugins:

```json
{"mcpServers":{"example":{"command":"bun","args":["C:/tools/server.ts"],"env":{"TOKEN":"${EXAMPLE_TOKEN}"}}}}
```

HTTP entries accept `url`, optional `type:"sse"`, and `headers` with `${VAR}`
expansion. Listing new workspace configuration does not execute commands.
`/mcp start <name|all>` explicitly starts/trusts it. Unchanged trusted configs
reconnect; changed configs require another start. `/mcp stop <name|all>` closes
transport/removes trust; `/mcp restart <name>` restarts. `/mcp`, `/mcp tools` and
`/tools` show connected tools. Plan allows only tools explicitly annotated
read-only by the server; that annotation is a server claim, not sandbox enforcement.

`/agent researcher <task>` (also planner/reviewer) runs a separate native worker
on the selected provider, with read-only native tools and no nested delegation.
Models can call `delegate`. There are four active slots, a five-handoff budget
per root turn, and the existing security-task routing guard. Status rows reflect
actual lifecycle events. `/stop` cancels workers/provider requests. Quitting
closes the owned control-plane process tree.

## Build and evidence

Use the pinned GNU Rust toolchain and repository WinLibs PATH. Bun is a runtime
prerequisite for extensions/delegation. Build/check:

```powershell
cd meg-rs
cargo test --locked --workspace
cargo build --locked --release --workspace
cd ../meg-plane
bun install --frozen-lockfile --ignore-scripts
bun test --timeout 20000
bun build src/index.ts --target=bun --outfile=dist/plane.js
cd ..
```

Distribution layout is `meg.exe`, `meg_ffi.dll`, `meg-plane/plane.js` and `meg-plane/skills/`
beside it, with Bun on PATH. `MEG_PLANE_ENTRY` overrides discovery. Source builds
also find the repository's `meg-plane/dist/plane.js` or `src/index.ts`.

Offline checks use loopback OpenAI/Anthropic providers, real Windows ConPTY,
Credential Manager, a real SDK MCP subprocess and native workers. Provider keys
and personal runtime configuration are not part of the distribution.
Remote MCP auth/OAuth, other live providers, cross-platform behavior and sustained
workloads still need deployment checks. These results do not imply universal
provider or plugin compatibility. The standalone product requires Rust and Bun,
not a separate evolution worker.
