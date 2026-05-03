//! `coral mcp serve [--transport stdio] [--read-only]`
//!
//! Exposes the wiki + manifest as a Model Context Protocol server.
//! v0.19 wave 2 ships stdio only; HTTP/SSE follows in v0.19.x once the
//! security model (audit log + rate limit) is pinned.

use anyhow::Result;
use clap::{Args, Subcommand};
use coral_mcp::{
    McpHandler, NoOpDispatcher, ResourceProvider, ServerConfig, ToolCallResult, ToolDispatcher,
    Transport, WikiResourceProvider,
};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCmd,
}

#[derive(Subcommand, Debug)]
pub enum McpCmd {
    /// Serve the MCP protocol over stdin/stdout.
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Transport. v0.19 only supports `stdio`.
    #[arg(long, default_value = "stdio")]
    pub transport: TransportArg,

    /// Default-deny: write-tools (`up`, `down`, `run_test`) are
    /// disabled unless `--allow-write-tools` is also passed.
    #[arg(long, default_value_t = true)]
    pub read_only: bool,

    /// Enable write tools. Mutually exclusive with `--read-only true`
    /// (clap's `default_value_t = true` means `--read-only false` is
    /// the explicit opt-out).
    #[arg(long)]
    pub allow_write_tools: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum TransportArg {
    Stdio,
}

pub fn run(args: McpArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        McpCmd::Serve(a) => serve(a),
    }
}

fn serve(args: ServeArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let resources: Arc<dyn ResourceProvider> = Arc::new(WikiResourceProvider::new(cwd));
    let tools: Arc<dyn ToolDispatcher> = Arc::new(NoOpDispatcher);
    let read_only = args.read_only && !args.allow_write_tools;
    let config = ServerConfig {
        transport: match args.transport {
            TransportArg::Stdio => Transport::Stdio,
        },
        read_only,
        port: None,
    };
    let handler = McpHandler::new(config, resources, tools);
    eprintln!(
        "coral mcp serve — transport={:?}, read_only={}",
        match args.transport {
            TransportArg::Stdio => "stdio",
        },
        read_only
    );
    handler
        .serve_stdio()
        .map_err(|e| anyhow::anyhow!("MCP stdio loop failed: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

#[allow(dead_code)]
fn _ensure_tool_call_result_used(_t: ToolCallResult) {}
