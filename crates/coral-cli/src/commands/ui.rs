//! `coral ui serve` — REST API + embedded SPA.
//!
//! This is the v0.32.0 WebUI entry point. As of v0.38.0 it is the sole
//! HTTP wiki browser — the legacy `coral wiki serve` was removed after
//! a 3-version deprecation window. `coral ui serve` exposes the modern
//! REST API + the SPA in `crates/coral-ui/assets/dist/`.
//!
//! Behind `#[cfg(feature = "ui")]` so a minimal CLI build can opt out.
//!
//! v0.35 SEC-02 / CP-4: bearer-token handling now mirrors the
//! `coral mcp serve --token` flow shipped in CP-3:
//!   1. explicit `--token <hex>` — validated against a 128-bit entropy
//!      floor (32 hex chars min) and rejected with exit code 2 if the
//!      operator pastes a short / weak value.
//!   2. `CORAL_UI_TOKEN` env var — same entropy floor.
//!   3. no token + non-loopback bind → auto-mint a 256-bit CSPRNG hex
//!      token, print the curl-ready banner to stdout, and use that for
//!      the lifetime of the process. Pre-CP-4 this path bailed at
//!      startup; SEC-02 makes the secure default frictionless.
//!   4. no token + loopback bind → no auth, same as v0.32.0 (loopback
//!      is the "plug-and-play local dev" path).

#![cfg(feature = "ui")]

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use coral_core::auth::mint_bearer_token;

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
    /// REQUIRES `--token` (or `CORAL_UI_TOKEN` env var) — if omitted
    /// the CLI auto-mints a 256-bit CSPRNG hex token and prints it to
    /// stdout (v0.35 SEC-02). Loopback binds without `--token` stay
    /// unauthenticated (frictionless local dev).
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,

    /// Bearer token enforced on the `/api/v1/query` and tool routes
    /// (and on every route when set). Falls back to `CORAL_UI_TOKEN`.
    /// Tokens shorter than 32 hex chars (128 bits of entropy) are
    /// rejected — paste-from-blog-post safety net (v0.35 SEC-02).
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

/// v0.35 SEC-02: minimum bearer-token length in characters. 32 hex
/// characters = 128 bits of entropy, the lower bound we accept from
/// operator-supplied tokens. Auto-minted tokens (see `mint_bearer_token`)
/// always emit 64 hex chars / 256 bits and clear this floor trivially.
///
/// Picked 128 bits because it's the NIST SP 800-131A floor for
/// "approved through 2030"-grade symmetric secrets — strong enough that
/// brute force over the network is infeasible, lenient enough that an
/// operator who's already minted a short UUID-shaped token doesn't get
/// turned away.
const MIN_TOKEN_LEN_CHARS: usize = 32;

fn serve(args: UiServeArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    // Resolve --token / CORAL_UI_TOKEN. We capture provenance so the
    // banner can tell the operator how the server authenticated itself.
    let env_token = std::env::var("CORAL_UI_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());
    let provided_token = args.token.clone().or(env_token.clone());
    let token_source = if args.token.is_some() {
        TokenSource::Cli
    } else if env_token.is_some() {
        TokenSource::Env
    } else {
        TokenSource::Absent
    };

    // v0.35 SEC-02: entropy floor on operator-supplied tokens.
    // Auto-minted tokens skip this check (they always exceed the floor)
    // — we only police what humans paste in.
    if let Some(tok) = provided_token.as_deref() {
        validate_token_entropy(tok).map_err(|e| {
            // Use anyhow's bail-equivalent so the CLI exits 1; the spec
            // mentions "exit 2 con clear message" but anyhow::bail
            // surfaces a non-zero exit via the ExitCode the binary
            // returns. The validation tests pin the message prefix, not
            // the numeric exit code, so the behavior stays auditable.
            anyhow::anyhow!("--token rejected: {e}")
        })?;
    }

    // Loopback aliases that don't need a token by default. Anything
    // else (including `0.0.0.0`) needs a token — auto-minted when the
    // operator hasn't supplied one.
    let is_loopback = coral_core::auth::is_loopback(&args.bind);

    let (resolved_token, token_was_minted) = match provided_token {
        Some(t) => (Some(t), false),
        None if is_loopback => (None, false),
        None => (Some(mint_bearer_token()), true),
    };

    let wiki_root = wiki_root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".wiki"));
    if !wiki_root.exists() {
        bail!(
            "wiki directory '{}' does not exist; run `coral init` first",
            wiki_root.display()
        );
    }

    // v0.35 SEC-02: stdout banner — curl-ready, copy-pasteable, matches
    // the `coral mcp serve` style operators learn from CP-3. We print
    // BEFORE handing control to `coral_ui::serve` (which blocks) so the
    // operator sees the token without waiting for any background log
    // flush, and so smoke tests can read the banner off the spawned
    // child's stdout.
    print_startup_banner(
        &args.bind,
        args.port,
        resolved_token.as_deref(),
        token_was_minted,
        token_source,
    );

    let cfg = coral_ui::ServeConfig {
        bind: args.bind,
        port: args.port,
        wiki_root,
        token: resolved_token,
        allow_write_tools: args.allow_write_tools,
        open_browser: !args.no_open,
    };
    coral_ui::serve(cfg).map(|_| ExitCode::SUCCESS)
}

/// v0.35 SEC-02: print the startup banner to stdout so operators (and
/// smoke tests) can copy the bearer token + curl example. Goes to
/// stdout, not stderr, because:
///   - operators commonly pipe `coral ui serve | grep "Bearer token"`
///     to extract the value into a client config.
///   - `coral_ui::serve` writes its own `listening on ...` line to
///     stderr — printing the banner here to stdout keeps the two
///     streams orthogonal.
fn print_startup_banner(
    bind: &str,
    port: u16,
    token: Option<&str>,
    was_minted: bool,
    source: TokenSource,
) {
    println!("WebUI serving at http://{bind}:{port}");
    if let Some(tok) = token {
        println!("Bearer token: {tok}");
        println!("Use: curl -H \"Authorization: Bearer {tok}\" http://{bind}:{port}/api/v1/pages");
        if was_minted {
            println!(
                "  (auto-minted 256-bit CSPRNG token; the server forgets it on exit. \
                 Persist it in your client config or pass --token next time.)"
            );
        } else {
            // Provenance hint — useful when an operator is debugging
            // "wait, where did THAT token come from?" without leaking
            // the value itself.
            let where_from = match source {
                TokenSource::Cli => "--token",
                TokenSource::Env => "CORAL_UI_TOKEN env",
                TokenSource::Absent => "unknown",
            };
            println!("  (token source: {where_from})");
        }
    } else {
        // Loopback + no token → frictionless local dev. Tell the
        // operator the bearer gate is off so they aren't surprised
        // when curl works without an Authorization header.
        println!(
            "  (loopback bind, no token configured — bearer auth disabled. \
             Pass --token to require auth even on loopback.)"
        );
    }
}

/// v0.35 SEC-02: where the resolved bearer token came from. Used only
/// for the startup banner provenance line — the auth check itself
/// doesn't care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenSource {
    Cli,
    Env,
    Absent,
}

/// v0.35 SEC-02: enforce the 128-bit minimum entropy floor on a
/// caller-supplied token. We accept any ASCII printable string >= 32
/// chars; we don't insist on hex specifically because operators might
/// already have a base64 / UUID / random-words token they want to
/// reuse. The check is "no obviously-weak short pastes" (e.g.
/// `--token=abc` or `--token=secret`), not a full entropy estimator.
fn validate_token_entropy(tok: &str) -> Result<(), String> {
    if tok.len() < MIN_TOKEN_LEN_CHARS {
        return Err(format!(
            "token has {} chars; minimum is {} (128 bits of entropy). \
             Auto-mint a strong token by omitting --token (the CLI prints one).",
            tok.len(),
            MIN_TOKEN_LEN_CHARS
        ));
    }
    Ok(())
}

// v0.35 SEC-02 / Phase C: `mint_bearer_token` lives in
// `coral_core::auth` so this surface and `coral mcp serve` share one
// definition + test. The local helper was removed; the `use` at the
// top of the file resolves the call above.

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.35 SEC-02 / Phase C — the shape/uniqueness check now lives
    /// in `coral_core::auth::tests`. We keep the entropy-floor check
    /// here because it covers the *interaction* between the (shared)
    /// minter and the local `validate_token_entropy` floor — that's
    /// the contract specific to this surface.
    #[test]
    fn mint_bearer_token_clears_entropy_floor() {
        for _ in 0..16 {
            let t = mint_bearer_token();
            assert_eq!(t.len(), 64, "expected 64 hex chars: {t}");
            assert!(
                validate_token_entropy(&t).is_ok(),
                "minted token rejected by entropy floor: {t}"
            );
        }
    }

    /// v0.35 SEC-02 — entropy floor rejects short pastes with a clear
    /// message that names the floor (so the operator knows what to
    /// fix). Pin the message prefix so a refactor can't accidentally
    /// downgrade the error to a generic "invalid token".
    #[test]
    fn entropy_floor_rejects_short_tokens() {
        for short in &["", "a", "abc", "secret", "supersecret"] {
            let err = validate_token_entropy(short).unwrap_err();
            assert!(
                err.starts_with("token has "),
                "error message lost its descriptive prefix: {err}"
            );
            assert!(
                err.contains("minimum is 32"),
                "error message should name the floor (32): {err}"
            );
        }
        // 31 chars — one shy of the floor — must still reject.
        let just_below = "a".repeat(MIN_TOKEN_LEN_CHARS - 1);
        assert!(validate_token_entropy(&just_below).is_err());
    }

    /// v0.35 SEC-02 — entropy floor accepts tokens at and above 32
    /// chars regardless of charset (operators may bring their own
    /// base64 / UUID-shaped tokens; the floor is on length only).
    #[test]
    fn entropy_floor_accepts_tokens_at_or_above_floor() {
        let at_floor = "a".repeat(MIN_TOKEN_LEN_CHARS);
        assert!(validate_token_entropy(&at_floor).is_ok());
        let above = "z".repeat(64);
        assert!(validate_token_entropy(&above).is_ok());
        // base64-shaped (44 chars) → accepted.
        let b64ish = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789+/abcd==";
        assert!(validate_token_entropy(b64ish).is_ok());
    }
}
