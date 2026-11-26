//! meg ? the native Rust coding harness binary.
//!
//! Entrypoints:
//! - Bare invocation (`meg`) or `meg tui`: opens the interactive Workstation TUI.
//! - `meg run "<goal>"`: runs the agent loop headless.
//! - `meg loop --check "<cmd>" "<goal>"`: runs the outer verification retry loop.
//! - `meg -h` / `meg --help`: displays CLI usage.

mod bridge;
mod cli;
mod engine;
mod loop_cmd;
mod runner;
mod tui_launch;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod native_integration;

use anyhow::Result;
use crate::bridge::run_ui_bridge;
use crate::cli::{CliCommand, parse_cli_args, parse_loop_options, parse_run_options, print_usage};
use crate::loop_cmd::run_loop_headless;
use crate::runner::{run_goal_headless, run_worker};
use crate::tui_launch::launch_tui;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let workspace = std::env::current_dir()?;

    match parse_cli_args(&args) {
        CliCommand::Worker => run_worker().await,
        CliCommand::Tui => {
            launch_tui(&workspace)?;
            Ok(())
        }
        CliCommand::Run { goal } => {
            let (_, options) = parse_run_options(&args)?;
            let workspace = options
                .cwd
                .as_deref()
                .unwrap_or(&workspace)
                .canonicalize()?;
            anyhow::ensure!(workspace.is_dir(), "Workspace must be a directory");
            run_goal_headless(&workspace, &goal, &options).await?;
            Ok(())
        }
        CliCommand::Loop => {
            let options = parse_loop_options(&args)?;
            let workspace = options
                .base
                .cwd
                .as_deref()
                .unwrap_or(&workspace)
                .canonicalize()?;
            anyhow::ensure!(workspace.is_dir(), "Workspace must be a directory");
            run_loop_headless(&workspace, &options).await?;
            Ok(())
        }
        CliCommand::UiBridge => {
            let mut ws = workspace;
            if let Some(dir) = args.iter().position(|a| a == "-C" || a == "--cd").and_then(|pos| args.get(pos + 1)) {
                ws = std::path::PathBuf::from(dir).canonicalize()?;
            }
            anyhow::ensure!(ws.is_dir(), "Workspace must be a directory");
            run_ui_bridge(&ws).await?;
            Ok(())
        }
        CliCommand::Help => {
            print_usage();
            Ok(())
        }
        CliCommand::Version => {
            println!("MEG {} (meg-tui)", meg_tui::APP_VERSION);
            Ok(())
        }
        CliCommand::Unknown(cmd) => {
            eprintln!("Unknown command: {cmd}\n");
            print_usage();
            std::process::exit(1);
        }
    }
}

