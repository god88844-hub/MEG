//! One command contract drives help, completion, validation and dispatch.
//! Local commands never fall through to model/tool execution.
use crate::{runtime::Controller, session::atomic_json, *};
use anyhow::{bail, Context, Result};

macro_rules! command_table {
    ($(($id:ident, $name:literal, $description:literal, $args:literal, $busy:literal)),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum CommandId { $($id),* }
        pub const COMMANDS: &[CommandSpec] = &[$(CommandSpec { id: CommandId::$id, name: $name, description: $description, accepts_args: $args, available_busy: $busy }),*];
        pub const ESSENTIAL_COMMANDS: &[(&str, &str)] = &[$(($name, $description)),*];
    }
}
pub struct CommandSpec {
    pub id: CommandId,
    pub name: &'static str,
    pub description: &'static str,
    pub accepts_args: bool,
    pub available_busy: bool,
}
command_table![
    (
        Help,
        "/help",
        "Commands and keyboard shortcuts",
        false,
        true
    ),
    (
        New,
        "/new",
        "Save this session and start a new conversation",
        false,
        false
    ),
    (
        Clear,
        "/clear",
        "Clear the visible transcript; keep conversation context",
        false,
        true
    ),
    (
        Sessions,
        "/sessions",
        "Browse saved sessions in this workspace",
        false,
        true
    ),
    (
        Resume,
        "/resume",
        "Resume the latest session or a session ID",
        true,
        false
    ),
    (
        Fork,
        "/fork",
        "Save a copy of this conversation as a new session",
        false,
        false
    ),
    (
        Export,
        "/export",
        "Export this conversation to a workspace file",
        true,
        true
    ),
    (
        Compact,
        "/compact",
        "Archive older context and keep recent messages",
        false,
        false
    ),
    (
        Copy,
        "/copy",
        "Copy the last assistant reply to the clipboard",
        false,
        true
    ),
    (
        Stats,
        "/stats",
        "Show provider-reported usage for this session",
        false,
        true
    ),
    (
        Model,
        "/model",
        "Choose a model from the endpoint's model roster",
        true,
        false
    ),
    (
        Effort,
        "/effort",
        "Set reasoning effort for the next request",
        true,
        false
    ),
    (
        Provider,
        "/provider",
        "Connect and manage local or cloud model providers",
        true,
        false
    ),
    (
        Skills,
        "/skills",
        "Browse and use installed workspace skills",
        true,
        false
    ),
    (
        Plugins,
        "/plugins",
        "Install, enable or remove local plugin packages",
        true,
        true
    ),
    (
        Tools,
        "/tools",
        "Inspect native and connected MCP tools",
        false,
        true
    ),
    (
        Config,
        "/config",
        "Inspect settings without revealing credentials",
        false,
        true
    ),
    (
        ContextInfo,
        "/context",
        "Inspect conversation and workspace context",
        false,
        true
    ),
    (
        Settings,
        "/settings",
        "Change connections, models, appearance and behavior",
        false,
        true
    ),
    (
        Init,
        "/init",
        "Ask MEG to create workspace guidance",
        true,
        false
    ),
    (
        Permissions,
        "/permissions",
        "Show or change Act / read-only Plan mode",
        true,
        false
    ),
    (
        Plan,
        "/plan",
        "Plan a task using read-only tools",
        true,
        false
    ),
    (Diff, "/diff", "Show the workspace Git diff", false, true),
    (
        Review,
        "/review",
        "Ask MEG to review workspace changes",
        true,
        false
    ),
    (
        Code,
        "/code",
        "Ask MEG to work on a coding task",
        true,
        false
    ),
    (
        Undo,
        "/undo",
        "Undo MEG's last recorded file mutation",
        false,
        false
    ),
    (
        Verify,
        "/verify",
        "Ask MEG to verify a task with evidence",
        true,
        false
    ),
    (
        Agent,
        "/agent",
        "Run a read-only planner, researcher or reviewer",
        true,
        true
    ),
    (
        Memory,
        "/memory",
        "Read, add or search workspace notes",
        true,
        true
    ),
    (
        Todo,
        "/todo",
        "View, add, complete or clear task items",
        true,
        true
    ),
    (
        Status,
        "/status",
        "Session, workspace, model and run status",
        false,
        true
    ),
    (
        Doctor,
        "/doctor",
        "Inspect session storage and endpoint configuration",
        false,
        true
    ),
    (Theme, "/theme", "Preview or choose a theme", true, true),
    (
        Mcp,
        "/mcp",
        "Connect MCP servers and inspect their tools",
        true,
        true
    ),
    (
        Workspace,
        "/workspace",
        "Choose a workspace or open a directory",
        true,
        false
    ),
    (
        Stop,
        "/stop",
        "Cancel the active model request and shell process tree",
        false,
        true
    ),
    (
        Queue,
        "/queue",
        "Inspect, clear or run queued follow-up prompts",
        true,
        true
    ),
    (
        Ui,
        "/ui",
        "Switch /ui classic|pro, toggle cards, or reload ~/.meg/ui.json",
        true,
        true
    ),
    (
        Pair,
        "/pair",
        "Configure single-model (/pair off) or multi-model orchestration",
        true,
        false
    ),    (
        Exit,
        "/exit",
        "Save the session, stop owned work and exit",
        false,
        true
    ),
];

pub fn lookup(name: &str) -> Option<&'static CommandSpec> {
    let canonical = match name {
        "/?" => "/help",
        "/quit" => "/exit",
        "/models" => "/model",
        "/connect" | "/login" => "/provider",
        "/skill" => "/skills",
        "/commands" => "/help",
        "/agents" => "/agent",
        "/themes" => "/theme",
        "/cancel" => "/stop",
        "/cd" => "/workspace",
        _ => name,
    };
    COMMANDS.iter().find(|c| c.name == canonical)
}

impl Controller {
    pub(crate) fn command(&mut self, text: &str) -> Result<Option<String>> {
        let (name, args) = text
            .trim()
            .split_once(char::is_whitespace)
            .unwrap_or((text.trim(), ""));
        let args = args.trim();
        if let Some(skill) = name.strip_prefix("/skill:") {
            return self.command(&format!("/skills {skill} {args}"));
        }
        if lookup(name).is_none() {
            if let Some(command) = self.state.extensions["commands"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|c| c["name"].as_str() == Some(name.trim_start_matches('/')))
                })
            {
                anyhow::ensure!(
                    !self.state.busy,
                    "Stop the active run before starting a plugin command"
                );
                let body = command["body"]
                    .as_str()
                    .context("Invalid plugin command")?
                    .replace("$ARGUMENTS", args);
                return Ok(Some(format!(
                    "User-selected plugin command {name}:\n{body}\nArguments: {args}"
                )));
            }
        }
        let spec = lookup(name).with_context(|| {
            format!("Unknown command '{name}'. Use /help. No model request was made.")
        })?;
        if self.state.busy && !spec.available_busy {
            bail!(
                "{} is unavailable during a run. Use /stop first.",
                spec.name
            );
        }
        if !args.is_empty() && !spec.accepts_args {
            bail!("{} does not accept arguments", spec.name);
        }
        use CommandId::*;
        match spec.id {
            New => { self.save()?; self.reset_session(); self.state.notice="New session.".into(); }
            Clear => { self.state.transcript.clear();self.state.transcript_scroll=0;self.state.notice="Transcript cleared; conversation context kept.".into(); }
            Sessions => self.open_sessions()?,
            Copy => { let reply=self.state.conversation.iter().rev().find(|m|m.role=="assistant").context("No assistant reply to copy")?;copy_clipboard(&reply.content)?;self.state.notice="Assistant reply copied.".into(); }
            Export => {
                let name=if args.is_empty(){format!(".meg/tui/exports/{}.txt",self.state.session_id)}else{args.to_string()};
                let path=meg_tools::fs_tools::resolve(std::path::Path::new(&self.state.cwd),&name)?;
                if let Some(parent)=path.parent(){std::fs::create_dir_all(parent)?;}
                std::fs::write(&path,self.state.transcript.join("\n\n"))?;self.system(format!("Exported session to {}",path.display()));
            }
            Compact => {
                if self.state.conversation.len()<=8{self.system("Context is already short; no compaction needed.");}
                else{
                    let store=self.store.as_ref().context("Persistent storage is required before compacting")?;
                    let archive=store.root.join(format!("context-{}.json",uuid::Uuid::new_v4()));
                    atomic_json(&archive,&self.state.conversation)?;
                    let older=self.state.conversation.len()-8;
                    let excerpts=self.state.conversation[..older].iter().map(|m|format!("{}: {}",m.role,m.content.chars().take(240).collect::<String>())).take(24).collect::<Vec<_>>().join("\n");
                    let mut context=vec![meg_core::ChatMessage{role:"user".into(),content:format!("Earlier conversation excerpts (truncated verbatim, not a semantic summary). Full archive: {}\n{excerpts}",archive.display())}];
                    context.extend_from_slice(&self.state.conversation[older..]);self.state.conversation=context;
                    self.system(format!("Archived {older} older messages and retained excerpts plus the latest 8. Full context: {}",archive.display()));
                }
            }
            Help => {
                self.system(COMMANDS.iter().map(|c| format!("{:<12} {}", c.name, c.description)).collect::<Vec<_>>().join("\n"));
                self.system("Enter send · Shift/Alt+Enter newline · Tab complete command · Ctrl+P commands\nEsc stop/close · Ctrl+C cancel/clear · Ctrl+D exit · Ctrl+T thinking · PgUp/PgDn scroll\n@path attaches a workspace text file to the next message.");
            }
            Resume => {
                let store = self.store.as_ref().context("Session storage unavailable")?;
                let id = if args.is_empty() { store.list()?.into_iter().find(|r| r.id != self.state.session_id).context("No earlier saved session")?.id } else { args.into() };
                let record = store.load(&id)?; self.save()?; record.restore(&mut self.state);
                self.state.notice = format!("Resumed session {}", self.state.session_id);
            }
            Fork => { self.save()?; self.state.session_id = uuid::Uuid::new_v4().to_string(); self.save()?; self.system("Conversation forked; the original session is preserved."); }
            Stats => {
                let u = &self.state.usage;
                let text=if u.reported_requests==0 {format!("Model requests: {}\nThe provider has not reported token usage. No token or cost estimate is shown.",u.requests)}else{format!("Model requests: {} · Usage reported for {}\nInput: {} · Output: {} · Cached input: {}\nCounts cover only requests with provider usage fields. Provider pricing is unavailable.", u.requests,u.reported_requests,u.input_tokens,u.output_tokens,u.cached_tokens)};
                self.system(text);
            }
            Model => {
                if args.is_empty() { self.state.open_model_picker(); self.refresh_model_catalog()?; }
                else if args=="refresh"{self.refresh_model_catalog()?; self.state.open_model_picker();}
                else if let Some((provider,model))=args.split_once("::"){let profile=self.state.providers.iter().find(|p|p.id==provider||p.name.eq_ignore_ascii_case(provider)).context("Unknown provider; use /provider")?.clone();self.use_provider(profile)?;self.state.select_model(model.into());}
                else if self.state.available_models.iter().any(|m| m == args) { self.state.select_model(args.into()); self.state.notice = format!("Model: {args}"); }
                else if self.state.provider.is_some()&&!args.chars().any(char::is_whitespace){self.state.select_model(args.into());self.state.notice=format!("Model: {args} (manual selection)");}
                else { bail!("Model '{args}' is absent from the endpoint roster. Use /model to inspect available models."); }
            }
            Effort => {
                if args.is_empty() { self.state.open_effort_picker(); }
                else if matches!(args,"low"|"medium"|"high"|"max") { self.state.select_effort(args.into()); self.state.notice=format!("Reasoning effort: {args}"); }
                else { bail!("Effort must be low, medium, high or max"); }
            }
            Provider => {
                if args.is_empty(){self.open_provider();}
                else if args=="add"{self.open_provider();}
                else if args=="add custom"{self.open_provider();self.state.provider_dialog.as_mut().unwrap().new_connection(meg_core::providers::ProviderKind::OpenAi);}
                else if let Some(preset)=args.strip_prefix("add "){self.open_provider();anyhow::ensure!(self.state.provider_dialog.as_mut().unwrap().new_preset(preset),"Unknown preset. Open /provider to search services, or use /provider add custom");}
                else if let Some(name)=args.strip_prefix("use "){let profile=self.state.providers.iter().find(|p|p.id==name||p.name.eq_ignore_ascii_case(name)).context("Unknown provider")?.clone();self.use_provider(profile)?;}
                else if args=="test"||args=="refresh"{let profile=self.state.provider.as_ref().context("Select a provider")?.clone();self.invoke_service("provider.models",serde_json::to_value(profile)?)?;}
                else{bail!("Usage: /provider [add [preset]|use <name>|test]");}
            }
            Skills=>{if args=="refresh"{self.invoke_service("runtime.catalog",serde_json::json!({}))?;}else if args.is_empty(){let items=self.state.extensions["skills"].as_array().cloned().unwrap_or_default();self.system(if items.is_empty(){"No skills loaded. Put SKILL.md in .meg/skills/<name>/, then /skills refresh.".into()}else{items.iter().map(|s|format!("{} — {}",s["name"].as_str().unwrap_or(""),s["description"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")});}else{let(name,task)=args.split_once(char::is_whitespace).unwrap_or((args,""));let skill=self.state.extensions["skills"].as_array().and_then(|items|items.iter().find(|s|s["name"].as_str()==Some(name))).context("Unknown skill; use /skills or /skills refresh")?;return Ok(Some(format!("Use this user-selected skill within current tool permissions:\n{}\nTask: {}",skill["body"].as_str().unwrap_or(""),if task.is_empty(){"Apply the workflow to the current workspace"}else{task})));}}
            Plugins=>self.invoke_service("plugins.command",serde_json::json!({"command":args}))?,
            Tools=>{let tools=meg_tools::ToolRegistry::with_core_tools(std::path::Path::new(&self.state.cwd));let mut rows=tools.stubs.iter().map(|(name,description)|format!("{name}: {description}")).collect::<Vec<_>>();if self.state.extensions["skills"].is_array(){rows.push("read_skill: load an installed skill; delegate: read-only research/review".into());}for tool in self.state.extensions["tools"].as_array().into_iter().flatten(){rows.push(format!("{}: {}",tool["alias"].as_str().unwrap_or(""),tool["description"].as_str().unwrap_or("")));}self.system(rows.join("\n"));},
            Config=>{let profile=self.state.provider.as_ref();self.system(format!("Settings: {}\nProvider: {}\nEndpoint: {}\nModel: {}\nTheme: {} · Effort: {} · Mode: {}\nUse /provider, /model, /theme, /effort, /permissions, /skills, /mcp and /plugins.",self.state.provider_root.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"unavailable".into()),profile.map(|p|p.name.as_str()).unwrap_or("none"),profile.map(|p|p.base_url.as_str()).unwrap_or("none"),self.state.model,self.state.theme.name,self.state.effort,self.state.mode.label()));},
            Settings=>{
                let choices=[
                    ("Connections","Cloud providers, API keys and local endpoints","/provider"),
                    ("Models","Choose across connected services and local servers","/model"),
                    ("Theme","Appearance and contrast","/theme"),
                    ("Reasoning effort","Model reasoning preference","/effort"),
                    ("Plan mode","Read-only tools","/permissions readonly"),
                    ("Act mode","Workspace editing and shell tools","/permissions act"),
                    ("Workspace","Open or create a project folder","/workspace"),
                    ("Skills","Discover installed workflows","/skills"),
                    ("MCP servers","Inspect and manage tool connections","/mcp"),
                    ("Plugins","Inspect and manage local extensions","/plugins"),
                    ("Effective configuration","Inspect public settings and paths","/config"),
                    ("Keyboard and commands","Editing, navigation and command help","/help"),
                ];
                self.state.settings_picker=Some(PickerState::new("Settings".into(),choices.into_iter().map(|(label,detail,command)|PickerItem{label:label.into(),detail:detail.into(),meta:String::new(),value:command.into()}).collect()));
            },
            ContextInfo=>{let bytes=self.state.conversation.iter().map(|m|m.content.len()).sum::<usize>();let guidance=std::path::Path::new(&self.state.cwd).join("AGENTS.md");let guidance=guidance.metadata().ok().filter(|m|m.is_file()&&m.len()<=64*1024).map(|m|format!("AGENTS.md ({} bytes)",m.len())).unwrap_or_else(||"No readable workspace AGENTS.md within the 64 KiB limit".into());let skills=self.state.extensions["skills"].as_array().map(Vec::len).unwrap_or(0);self.system(format!("Conversation: {} messages · {bytes} UTF-8 bytes\nWorkspace: {}\nGuidance: {guidance}\nDiscovered skills: {skills}\nUse @path to attach a workspace text file. Provider context limits are not inferred.",self.state.conversation.len(),self.state.cwd));},
            Init=>{self.state.mode=Mode::Build;return Ok(Some(format!("Inspect this workspace and create or carefully improve AGENTS.md with accurate build, test, architecture and contribution guidance. Preserve existing project requirements and do not include credentials. Verify all commands you document. {args}")));},
            Permissions=>{match args{""=>{},"plan"|"readonly"=>self.state.mode=Mode::Plan,"act"|"build"=>self.state.mode=Mode::Build,_=>bail!("Usage: /permissions [readonly|act]")};self.system(format!("{} mode. Plan permits read-only native tools and annotated read-only MCP tools; Act permits workspace edits and shell commands.",self.state.mode.label()));},
            Plan => {
                self.state.mode = Mode::Plan;
                if args.is_empty() { self.system("Plan mode: native read tools, todo, skills, read-only delegates and read-only annotated MCP tools are available. /code switches to Act mode."); }
                else { return Ok(Some(format!("Produce a concrete implementation plan. Use read-only tools. Task: {args}"))); }
            }
            Diff => {
                let output = meg_tools::bash::run(std::path::Path::new(&self.state.cwd), &serde_json::json!({"command":"git --no-pager diff --no-ext-diff && git status --short", "timeout_seconds":10}))?;
                self.system(output);
            }
            Review | Verify | Code => {
                if spec.id == Code { self.state.mode = Mode::Build; }
                let instruction = match spec.id { Review => "Review the current workspace changes and report concrete findings with file locations.", Verify => "Verify the requested task using tests and observable evidence; report any failures.", _ => "Implement the requested coding task in the workspace and verify the changes." };
                if spec.id == Code && args.is_empty() { self.system("Act mode: file edits and shell tools are enabled. Enter a task to begin.");return Ok(None); }
                return Ok(Some(format!("{instruction}\n{args}")));
            }
            Undo => { let result = meg_tools::journal::undo(std::path::Path::new(&self.state.cwd))?; self.system(result); }
            Agent=>{if args.is_empty(){self.system(format!("Main agent: {}\n{}\nRun /agent <planner|researcher|reviewer> <task> for a real read-only worker.",if self.state.busy{"running"}else{"idle"},self.state.subagents.iter().map(|(n,s)|format!("{n}: {s}")).collect::<Vec<_>>().join("\n")));}else{let(role,task)=args.split_once(char::is_whitespace).context("Usage: /agent <planner|researcher|reviewer> <task>")?;let request=self.state.run_request(task.into());self.invoke_service("agents.run",serde_json::json!({"role":role,"task":task,"worker":{"provider":request.provider,"model":request.model,"base_url":request.base_url,"effort":request.effort}}))?;}}
            Memory => {
                let path = self.store.as_ref().context("Workspace storage unavailable")?.root.join("notes.json");
                let mut notes: Vec<String> = if path.exists() { serde_json::from_slice(&std::fs::read(&path)?)? } else { Vec::new() };
                if let Some(note) = args.strip_prefix("add ") { notes.push(note.into()); atomic_json(&path,&notes)?; self.system("Workspace note saved."); }
                else { let query = args.strip_prefix("search ").unwrap_or(args).to_lowercase(); self.system(notes.iter().filter(|n| n.to_lowercase().contains(&query)).map(|n| format!("• {n}")).collect::<Vec<_>>().join("\n")); }
            }
            Todo => {
                if let Some(task) = args.strip_prefix("add ") { self.state.plan_steps.push((false,task.into())); }
                else if let Some(index) = args.strip_prefix("done ") { let index: usize=index.parse().context("Usage: /todo done <1-based index>")?; self.state.plan_steps.get_mut(index.checked_sub(1).context("Index starts at 1")?).context("Todo index out of range")?.0=true; }
                else if args == "clear" { self.state.plan_steps.clear(); }
                else if !args.is_empty() { bail!("Usage: /todo [add <task>|done <index>|clear]"); }
                self.system(if self.state.plan_steps.is_empty() { "No task items.".into() } else { self.state.plan_steps.iter().enumerate().map(|(i,(d,s))| format!("{}. [{}] {s}",i+1,if *d {"x"} else {" "})).collect::<Vec<_>>().join("\n") });
            }
            Status | Doctor => self.system(format!("Session: {}\nWorkspace: {}\nModel: {} · effort {} · mode {}\nRun: {} · Endpoint last checked: {}\nSession store: {}",self.state.session_id,self.state.cwd,self.state.model,self.state.effort,self.state.mode.label(),if self.state.busy {"running"} else {"idle"},if self.state.online {"reachable"} else {"offline"},self.store.as_ref().map(|s|s.root.display().to_string()).unwrap_or_else(||"unavailable".into()))),
            Theme => { if args.is_empty() { self.state.open_theme_picker(); } else { self.state.theme=get_palette(args).context("Unknown theme; use /theme")?; self.state.notice=format!("Theme: {args}"); } }
            Mcp => {
                self.invoke_service("mcp.command",serde_json::json!({"command":args}))?;
            }
            Workspace => { if args.is_empty() { self.state.workspace_picker=Some(WorkspacePicker::open(self.state.cwd.clone().into())); } else { self.change_workspace(std::path::Path::new(args.trim_matches('"')))?; } }
            Stop => self.cancel(),
            Queue => {
                if args=="clear"{self.queue.clear();self.state.queued_count=0;self.state.notice="Follow-up queue cleared.".into();}
                else if args=="run"{anyhow::ensure!(!self.state.busy,"A run is already active");let(body,display)=self.queue.pop_front().context("Follow-up queue is empty")?;self.state.queued_count=self.queue.len();self.start_named(body,display)?;}
                else if args.is_empty(){self.system(if self.queue.is_empty(){"Follow-up queue is empty.".into()}else{self.queue.iter().enumerate().map(|(i,(_,display))|format!("{}. {display}",i+1)).collect::<Vec<_>>().join("\n")});}
                else{bail!("Usage: /queue [clear|run]");}
            }
            Ui => {
                let sub = args.trim().to_ascii_lowercase();
                if sub.is_empty() || sub == "status" || sub == "config" {
                    let ws_path = std::path::PathBuf::from(&self.state.cwd);
                    let global_p = ui_config::UserUiConfig::global_path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "~/.meg/ui.json".into());
                    self.system(format!(
                        "UI mode: {} | cards: {} | sidebar: {} | CSI-2026: {}\nCustom UI config paths:\n  workspace: {}\n  global   : {}",
                        self.state.ui_mode.label(),
                        if self.state.collapse_tool_cards { "compact" } else { "expanded" },
                        if self.state.ui_config.show_sidebar { "on" } else { "off" },
                        if self.state.ui_config.sync_csi_2026 { "on" } else { "off" },
                        ui_config::UserUiConfig::workspace_path(&ws_path).display(),
                        global_p
                    ));
                } else if let Some(mode) = ui_config::UiMode::parse(&sub) {
                    self.state.ui_mode = mode;
                    self.state.ui_config.ui_mode = mode;
                    self.state.notice = format!("UI mode: {}", mode.label());
                } else if sub == "cards" {
                    self.state.collapse_tool_cards = !self.state.collapse_tool_cards;
                    self.state.ui_config.compact_tool_cards = self.state.collapse_tool_cards;
                    self.state.notice = format!(
                        "Tool cards: {}",
                        if self.state.collapse_tool_cards { "compact" } else { "expanded" }
                    );
                } else if sub == "sidebar" {
                    self.state.ui_config.show_sidebar = !self.state.ui_config.show_sidebar;
                    self.state.notice = format!(
                        "Sidebar: {}",
                        if self.state.ui_config.show_sidebar { "on" } else { "off" }
                    );
                } else if sub == "reload" {
                    let ws_path = std::path::PathBuf::from(&self.state.cwd);
                    let (cfg, loaded_from) = ui_config::UserUiConfig::load(Some(&ws_path));
                    self.state.ui_mode = cfg.ui_mode;
                    self.state.collapse_tool_cards = cfg.compact_tool_cards;
                    if let Some(tname) = &cfg.theme_name {
                        if let Some(p) = get_palette(tname) {
                            self.state.theme = p;
                        }
                    }
                    self.state.pair_mode = cfg.pair_mode.clone();
                    self.state.executor_model = cfg.executor_model.clone();
                    self.state.reviewer_model = cfg.reviewer_model.clone();
                    self.state.ui_config = cfg;
                    self.state.notice = match loaded_from {
                        Some(p) => format!("Reloaded UI config from {}", p.display()),
                        None => "UI config reset to defaults (no ui.json found).".into(),
                    };
                } else {
                    bail!("Usage: /ui [classic|pro|cards|sidebar|reload|config]");
                }
            }
            Pair => {
                let parts: Vec<&str> = args.split_whitespace().collect();
                if parts.is_empty() || parts[0].eq_ignore_ascii_case("status") {
                    self.system(format!(
                        "Orchestration: {} | planner: {} | executor: {} | L3 reviewer: {}",
                        self.state.pair_mode.as_deref().unwrap_or("single_model"),
                        self.state.model,
                        self.state.executor_model.as_deref().unwrap_or("(none)"),
                        self.state.reviewer_model.as_deref().unwrap_or("(none)"),
                    ));
                } else if matches!(parts[0].to_ascii_lowercase().as_str(), "off" | "single" | "none") {
                    self.state.pair_mode = None;
                    self.state.executor_model = None;
                    self.state.reviewer_model = None;
                    self.state.notice = "Orchestration: single-model mode.".into();
                } else if matches!(parts[0], "devin_fusion" | "antigravity_orchestrator") {
                    let exec = parts
                        .get(1)
                        .copied()
                        .context("Usage: /pair <devin_fusion|antigravity_orchestrator> <executor-model> [reviewer-model]")?;
                    let rev = parts.get(2).map(|s| (*s).to_string());
                    self.state.pair_mode = Some(parts[0].to_string());
                    self.state.executor_model = Some(exec.to_string());
                    self.state.reviewer_model = rev.clone();
                    self.state.notice = format!(
                        "Orchestration: {} (exec={exec}{})",
                        parts[0],
                        rev.map(|r| format!(", L3={r}")).unwrap_or_default()
                    );
                } else {
                    bail!("Usage: /pair [off|status|devin_fusion <exec> [rev]|antigravity_orchestrator <exec> [rev]]");
                }
            }            Exit => { self.cancel(); self.state.should_quit=true; }
        }
        self.save()?;
        Ok(None)
    }
}

fn copy_clipboard(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("powershell.exe");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::InputEncoding = [System.Text.UTF8Encoding]::new(); Set-Clipboard -Value ([Console]::In.ReadToEnd())",
        ]);
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
        c
    };
    #[cfg(not(windows))]
    let mut command = Command::new("wl-copy");
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .context("Clipboard stdin unavailable")?
        .write_all(text.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "Clipboard unavailable: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
