//! `coral ui serve` — REST API + embedded SPA.
//!
//! This is the v0.32.0 WebUI entry point. The legacy `coral wiki serve`
//! command remains intact (BC); `coral ui serve` is the new, structured
//! API surface that the SPA in `crates/coral-ui/assets/dist/` consumes.
//!
//! Behind `#[cfg(feature = "ui")]` so a minimal CLI build can opt out.

#![cfg(feature = "ui")]

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct UiArgs {
    #[command(subcommand)]
    pub command: UiCmd,
}

#[derive(Subcommand, Debug)]
pub enum UiCmd {
    /// Start the WebUI server (REST API + embedded SPA).
    Serve(UiServeArgs),
}

#[derive(Args, Debug, Clone)]
pub struct UiServeArgs {
    /// Port to listen on (default: 3838).
    #[arg(long, default_value = "3838")]
    pub port: u16,

    /// Bind address (default: 127.0.0.1). Any non-loopback bind
    /// REQUIRES `--token` (or `CORAL_UI_TOKEN` env var).
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// Bearer token enforced on the `/api/v1/query` and tool routes
    /// (and on every route when set). Falls back to `CORAL_UI_TOKEN`.
    #[arg(long)]
    pub token: Option<String>,

    /// Skip the automatic browser launch at startup.
    #[arg(long)]
    pub no_open: bool,

    /// Enable write-tool routes (currently a stub — disabled by default).
    #[arg(long)]
    pub allow_write_tools: bool,
}

pub fn run(args: UiArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        UiCmd::Serve(s) => serve(s, wiki_root),
    }
}

fn serve(args: UiServeArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let token = args.token.or_else(|| std::env::var("CORAL_UI_TOKEN").ok());

    // Loopback aliases that don't need a token by default. Anything
    // else (including `0.0.0.0`) requires explicit auth.
    let is_loopback = matches!(args.bind.as_str(), "127.0.0.1" | "localhost" | "::1");
    if !is_loopback && token.is_none() {
        bail!(
            "binding to non-loopback address ({}) requires --token or CORAL_UI_TOKEN env var",
            args.bind
        );
    }

    let wiki_root = wiki_root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".wiki"));
    if !wiki_root.exists() {
        bail!(
            "wiki directory '{}' does not exist; run `coral init` first",
            wiki_root.display()
        );
    }

    let cfg = coral_ui::ServeConfig {
        bind: args.bind,
        port: args.port,
        wiki_root,
        token,
        allow_write_tools: args.allow_write_tools,
        open_browser: !args.no_open,
    };
    coral_ui::serve(cfg).map(|_| ExitCode::SUCCESS)
}
