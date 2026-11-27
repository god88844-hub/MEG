use anyhow::Context;
use anyhow::Result;
use meg_core::shim_client::ShimClient;
use meg_tui::RunnerMsg;
use meg_tui::WorkstationState;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use crate::cli::parse_tui_options;
use crate::engine::forward_agent_event;
use crate::runner::{RunExtras, detect_git_branch, meg_tui_clock, run_request_with_options, start_plane};

/// Launches the interactive Workstation with the selected local/cloud provider.
pub(crate) fn launch_tui(workspace: &Path) -> Result<()> {
    let options = parse_tui_options(&std::env::args().collect::<Vec<_>>())?;
    let workspace_path = options
        .cwd
        .clone()
        .unwrap_or_else(|| workspace.to_path_buf())
        .canonicalize()?;
    let workspace = workspace_path.as_path();
    let provider_store = meg_core::providers::ProviderStore::global();
    let settings = provider_store.load()?;
    let mut profiles = settings.profiles;
    let provider = if std::env::var_os("MEG_TUI_BASE_URL").is_some() {
        let p = meg_core::providers::ProviderProfile::environment();
        profiles.insert(0, p.clone());
        Some(p)
    } else {
        profiles
            .iter()
            .find(|p| p.id == settings.active)
            .or_else(|| profiles.first())
            .cloned()
    };
    let model = provider
        .as_ref()
        .map(|p| p.default_model.clone())
        .unwrap_or_default();
    let available_models = if model.is_empty() {
        Vec::new()
    } else {
        vec![model.clone()]
    };
    let show_picker = options.picker.unwrap_or(
        options.cwd.is_none()
            && options.resume.is_none()
            && std::env::var("MEG_NO_PICKER").ok().as_deref() != Some("1"),
    );
    if options.delegent {
        std::env::set_var("MEG_DELEGENT", "1");
    }
    if let Some(spec) = &options.subagent_model {
        std::env::set_var("MEG_SUBAGENT_MODEL", spec);
    }
    let plane_result = start_plane();
    let plane_error = plane_result.as_ref().err().map(ToString::to_string);
    let plane = plane_result.ok();
    let mut st = WorkstationState {
        provider,
        providers: profiles,
        provider_root: Some(provider_store.root),
        notice: plane_error
            .map(|e| format!("Control plane: {e}"))
            .unwrap_or_default(),
        startup: meg_tui::StartupOptions {
            resume: options.resume,
            model: options.model,
            effort: options.effort,
        },
        project: workspace
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        model,
        cwd: meg_tui::friendly_path(workspace),
        git_branch: detect_git_branch(workspace),
        clock: meg_tui_clock(),
        theme: meg_tui::DASHBOARD,
        online: false,
        available_models,
        workspace_picker: if show_picker {
            Some(meg_tui::WorkspacePicker::open_startup(
                workspace.to_path_buf(),
            ))
        } else {
            None
        },
        ..Default::default()
    };
    // The goal runner: shim agent loop per goal on a background thread
    // (its own tokio runtime), streaming deltas into the TUI.
    let goal_plane = plane.clone();
    let runner = Arc::new(
        move |request: meg_tui::RunRequest, tx: Sender<RunnerMsg>| -> Result<Option<String>> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let extras = RunExtras {
                pair_mode: request.pair_mode.clone(),
                executor_model: request.executor_model.clone(),
                reviewer_model: request.reviewer_model.clone(),
                ..RunExtras::default()
            };
            let (_outcomes, final_answer) = rt.block_on(run_request_with_options(
                request,
                Some(tx),
                goal_plane.clone(),
                extras,
            ))?;
            Ok(final_answer)
        },
    );
    let services: Arc<meg_tui::ServiceRunner> = Arc::new(move |request, cancel, events| {
        if request.method == "provider.models" {
            let profile: meg_core::providers::ProviderProfile =
                serde_json::from_value(request.params)?;
            let client = ShimClient::with_profile(&profile)?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let timeout =
                std::time::Duration::from_secs(if meg_core::provider_catalog::is_local(&profile) {
                    3
                } else {
                    15
                });
            let models=runtime.block_on(async{tokio::select!{result=client.models_with_timeout(timeout)=>result,_=cancel.cancelled()=>anyhow::bail!("Connection check cancelled")}})?;
            return Ok(
                serde_json::json!({"provider_id":profile.id,"base_url":profile.base_url,"models":models,"message":format!("Connected to {}",profile.name)}),
            );
        }
        let plane = plane
            .as_ref()
            .context("MEG native service runtime unavailable")?;
        let mut params = request.params;
        params["workspace"] = serde_json::to_value(request.workspace)?;
        plane.call_with_events(&request.method, params, &cancel, |value| {
            forward_agent_event(&Some(events.clone()), value)
        })
    });
    let (ui_cfg, _) = meg_tui::UserUiConfig::load(Some(workspace));
    st.ui_mode = options
        .ui_mode
        .as_deref()
        .and_then(meg_tui::UiMode::parse)
        .unwrap_or(ui_cfg.ui_mode);
    st.collapse_tool_cards = ui_cfg.compact_tool_cards;
    if let Some(tname) = &ui_cfg.theme_name {
        if let Some(p) = meg_tui::get_palette(tname) {
            st.theme = p;
        }
    }
    st.pair_mode = options.pair_mode.clone().or_else(|| ui_cfg.pair_mode.clone());
    st.executor_model = options
        .executor_model
        .clone()
        .or_else(|| ui_cfg.executor_model.clone());
    st.reviewer_model = options
        .reviewer_model
        .clone()
        .or_else(|| ui_cfg.reviewer_model.clone());
    st.ui_config = ui_cfg;
    meg_tui::run_app_with_services(st, Some(runner), Some(services))?;
    Ok(())
}

