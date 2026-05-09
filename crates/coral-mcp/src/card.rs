//! Discoverable MCP server card per the 2025-11-25 spec.
//!
//! v0.22.5 publishes a self-describing JSON document at
//! `GET /.well-known/mcp/server-card.json` (HTTP/SSE transport) and via
//! the `coral mcp card` CLI subcommand. Both surfaces emit byte-identical
//! JSON modulo the trailing newline `println!` adds. The card is the
//! discovery payload registries (and curious humans) hit before deciding
//! to connect — so it intentionally surfaces only what the spec defines
//! plus an `x-coral` namespace for Coral-specific fields.
//!
//! Schema (v0.22.5):
//!
//! ```json
//! {
//!   "name": "coral",
//!   "version": "<env!CARGO_PKG_VERSION>",
//!   "protocolVersion": "<server::PROTOCOL_VERSION>",
//!   "transports": ["stdio", "http"],
//!   "capabilities": {
//!     "resources": { "count": <N> },
//!     "tools":     { "count": <M> },
//!     "prompts":   { "count": <P> }
//!   },
//!   "vendor": {
//!     "name": "Coral",
//!     "url": "https://github.com/agustincbajo/Coral",
//!     "documentation": "https://github.com/agustincbajo/Coral#readme"
//!   },
//!   "x-coral": {
//!     "buildTimestamp": "<option_env!CORAL_BUILD_TIMESTAMP or 'unknown'>",
//!     "ciStatus": "green"
//!   }
//! }
//! ```
//!
//! Design constraints:
//! - `version` reads from `CARGO_PKG_VERSION` so a single bump in
//!   `Cargo.toml` flows through to the discovery payload — never two
//!   sources of truth.
//! - `protocolVersion` reuses [`crate::server::PROTOCOL_VERSION`] —
//!   refuses to drift apart from the JSON-RPC `initialize` reply.
//! - `buildTimestamp` is `option_env!`, not an `env!` requirement — a
//!   plain `cargo build` produces `"unknown"`. CI / reproducible builds
//!   may export `CORAL_BUILD_TIMESTAMP=<ISO-8601>` to populate it.
//!   No build-script changes (zero new deps).
//! - `ciStatus` is the literal `"green"` because the binary itself is
//!   the artifact CI just blessed — if the user has the binary, it
//!   passed CI. (A `"yellow"` / `"red"` status would lie about the
//!   provenance of the running binary.)
//! - `transports` is a literal `["stdio", "http"]` — Coral ships both
//!   unconditionally since v0.21.1.

use crate::prompts::PromptCatalog;
use crate::resources::ResourceProvider;
use crate::server::PROTOCOL_VERSION;
use crate::tools::ToolCatalog;

/// Build the discovery card. Capability counts are sampled from the
/// caller-supplied catalogs / provider so the HTTP route and the CLI
/// subcommand observe the same numbers — neither surface re-counts on
/// its own, which would risk drift if a future tool addition lands in
/// only one place.
///
/// Returns a `serde_json::Value` so the caller picks pretty / compact
/// rendering (the HTTP route uses `to_string_pretty`; the CLI prints
/// pretty with a trailing newline).
pub fn server_card(
    resources: &dyn ResourceProvider,
    tools: &ToolCatalog,
    prompts: &PromptCatalog,
) -> serde_json::Value {
    let resources_count = resources.list().len();
    // Tools: report the FULL catalog count (read-only + write) so the
    // discovery payload reflects what the server can do, not what the
    // current ServerConfig has gated. A registry deciding whether to
    // surface Coral cares about capability shape, not the per-process
    // permission knob.
    let tools_count = tools.all_count();
    let prompts_count = prompts.list_count();

    let build_timestamp = option_env!("CORAL_BUILD_TIMESTAMP").unwrap_or("unknown");

    serde_json::json!({
        "name": "coral",
        "version": env!("CARGO_PKG_VERSION"),
        "protocolVersion": PROTOCOL_VERSION,
        "transports": ["stdio", "http"],
        "capabilities": {
            "resources": { "count": resources_count },
            "tools": { "count": tools_count },
            "prompts": { "count": prompts_count },
        },
        "vendor": {
            "name": "Coral",
            "url": "https://github.com/agustincbajo/Coral",
            "documentation": "https://github.com/agustincbajo/Coral#readme",
        },
        "x-coral": {
            "buildTimestamp": build_timestamp,
            "ciStatus": "green",
        },
    })
}

/// Helper trait extension used by `server_card` so we don't leak
/// `ToolCatalog::all` / `PromptCatalog::list` allocations into every
/// call. Lives at module scope (not impl block) because `ToolCatalog`
/// is a unit struct in `tools.rs`.
impl ToolCatalog {
    /// Total tools across read-only + write surfaces. The card reports
    /// the whole catalog so registry consumers see capability shape,
    /// independent of any per-process `--allow-write-tools` gate.
    pub fn all_count(&self) -> usize {
        Self::all().len()
    }
}

impl PromptCatalog {
    /// Total prompts in the catalog. Mirrors `ToolCatalog::all_count`.
    pub fn list_count(&self) -> usize {
        Self::list().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::WikiResourceProvider;

    /// Construct a fresh catalog/provider triplet for the unit tests.
    /// `WikiResourceProvider::new("/tmp/coral-card-tests-empty")` reads
    /// no files (the wiki dir doesn't exist) so `list()` returns just
    /// the static catalog — deterministic across test machines.
    fn fixtures() -> (WikiResourceProvider, ToolCatalog, PromptCatalog) {
        let resources =
            WikiResourceProvider::new(std::path::PathBuf::from("/tmp/coral-card-tests-empty"));
        (resources, ToolCatalog, PromptCatalog)
    }

    /// AC #3 + spec D1: the card declares `name`, `version`, and
    /// `protocolVersion` at the top level with the correct values.
    #[test]
    fn card_has_name_version_protocolversion() {
        let (r, t, p) = fixtures();
        let card = server_card(&r, &t, &p);
        assert_eq!(card["name"], "coral", "card.name must be 'coral'");
        assert_eq!(
            card["version"],
            env!("CARGO_PKG_VERSION"),
            "card.version must match CARGO_PKG_VERSION"
        );
        assert_eq!(
            card["protocolVersion"], PROTOCOL_VERSION,
            "card.protocolVersion must reuse server::PROTOCOL_VERSION (no string duplication)"
        );
        // `transports` is the literal advertised list. A client looking
        // at this should know both stdio and http are wired in this
        // build (v0.21.1+).
        let transports = card["transports"].as_array().expect("transports array");
        let labels: Vec<&str> = transports
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect();
        assert!(labels.contains(&"stdio"));
        assert!(labels.contains(&"http"));
    }

    /// AC #4: `capabilities.{resources,tools,prompts}.count` match the
    /// catalog `.len()`. This is the load-bearing test — if a future
    /// tool addition lands in only the catalog and not the discovery
    /// path (or vice versa), this fails.
    #[test]
    fn card_capabilities_counts_match_catalog_lens() {
        let (r, t, p) = fixtures();
        let card = server_card(&r, &t, &p);
        let resources_count = card["capabilities"]["resources"]["count"]
            .as_u64()
            .expect("resources.count is integer");
        let tools_count = card["capabilities"]["tools"]["count"]
            .as_u64()
            .expect("tools.count is integer");
        let prompts_count = card["capabilities"]["prompts"]["count"]
            .as_u64()
            .expect("prompts.count is integer");
        assert_eq!(
            resources_count as usize,
            r.list().len(),
            "card resources count must equal provider.list().len()"
        );
        assert_eq!(
            tools_count as usize,
            ToolCatalog::all().len(),
            "card tools count must equal ToolCatalog::all().len()"
        );
        assert_eq!(
            prompts_count as usize,
            PromptCatalog::list().len(),
            "card prompts count must equal PromptCatalog::list().len()"
        );
    }

    /// AC #5/#6 wire-shape: the card serializes to pretty-printed JSON
    /// (multi-line, 2-space indent) AND parses back to the same value.
    /// This pins the exact wire format both surfaces emit so they can
    /// be byte-compared.
    #[test]
    fn card_serializes_to_pretty_json() {
        let (r, t, p) = fixtures();
        let card = server_card(&r, &t, &p);
        let pretty = serde_json::to_string_pretty(&card).expect("pretty JSON serialization");
        // Pretty form is multi-line and indented (4 lines minimum:
        // `{`, at least 2 nested fields each on own line, `}`).
        assert!(
            pretty.contains('\n'),
            "pretty JSON must be multi-line: {pretty:?}"
        );
        assert!(
            pretty.contains("  \"name\": \"coral\""),
            "pretty JSON must use 2-space indent for top-level fields: {pretty:?}"
        );
        // Round-trip equality: parsing the pretty string yields a
        // value identical to the input. If a future refactor
        // introduces non-determinism (e.g. a HashMap-iteration ordering
        // bug), this fires.
        let round_trip: serde_json::Value =
            serde_json::from_str(&pretty).expect("pretty JSON parses back");
        assert_eq!(round_trip, card);
        // Spec D1: `x-coral` namespace exists and carries the two
        // expected fields. `ciStatus` is the literal "green" because
        // the binary IS the CI-blessed artifact.
        assert_eq!(card["x-coral"]["ciStatus"], "green");
        let ts = card["x-coral"]["buildTimestamp"]
            .as_str()
            .expect("buildTimestamp is string");
        assert!(
            !ts.is_empty(),
            "buildTimestamp must be non-empty (got {ts:?})"
        );
    }
}
