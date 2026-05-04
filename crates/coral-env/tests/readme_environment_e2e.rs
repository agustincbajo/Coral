//! v0.19.5 audit C8: deeper readme-example test.
//!
//! `coral-core::tests::readme_examples_parse::readme_environment_healthcheck_subtable_example_parses`
//! checks that the README's `[[environments]]` block parses as TOML
//! and exposes a `services` table at the right path. This test goes
//! one step further — it deserializes the block all the way to
//! `EnvironmentSpec`, which is what `coral up` actually does at
//! runtime. Without this gate, a README that round-trips through
//! `parse_toml` but trips on `try_into::<EnvironmentSpec>` would
//! still ship with broken docs.

use coral_core::project::manifest::parse_toml;
use coral_env::EnvironmentSpec;
use std::path::Path;

const README_ENVIRONMENT_E2E: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com/api.git"

[[environments]]
name            = "dev"
backend         = "compose"
mode            = "managed"
compose_command = "auto"
production      = false

[environments.services.api]
kind  = "real"
repo  = "api"
ports = [3000]

[environments.services.api.healthcheck]
kind          = "http"
path          = "/health"
expect_status = 200

[environments.services.api.healthcheck.timing]
interval_s     = 2
timeout_s      = 5
retries        = 5
start_period_s = 30
"#;

#[test]
fn readme_environment_block_deserializes_into_environment_spec() {
    let manifest = parse_toml(README_ENVIRONMENT_E2E, Path::new("/tmp/coral.toml"))
        .expect("README environment example must parse");
    manifest.validate().expect("must validate");
    assert_eq!(manifest.environments_raw.len(), 1);

    let raw = manifest.environments_raw[0].clone();
    let spec: EnvironmentSpec = raw
        .try_into()
        .expect("EnvironmentSpec must deserialize from the README block");

    assert_eq!(spec.name, "dev");
    assert_eq!(spec.backend, "compose");
    assert_eq!(spec.services.len(), 1);
    assert!(spec.services.contains_key("api"));
}

/// Snapshot of the README.md at compile time. We read EVERY toml
/// fenced code block that contains an `[[environments]]` declaration
/// and assert each one round-trips through `EnvironmentSpec`. This
/// guards against the v0.19.5 audit C8 partial-fix bug — the audit
/// agent fixed the first README example block but missed the second
/// one ("Reference: Full coral.toml") that re-introduced the same
/// `[environments.dev.services.*]` shape. The validator caught it at
/// review time but the inline-literal `READMME_ENVIRONMENT_E2E` above
/// only tests a hand-written example, so it can't surface README
/// drift on its own. This test reads the actual README and refuses
/// any `[[environments]]` block that won't deserialize.
const README_MARKDOWN: &str = include_str!("../../../README.md");

#[test]
fn every_toml_environments_block_in_readme_deserializes() {
    let blocks = extract_toml_blocks_with_environments(README_MARKDOWN);
    assert!(
        !blocks.is_empty(),
        "expected at least one ```toml [[environments]] block in README; \
         did the layout change?"
    );

    for (block_idx, raw_block) in blocks.iter().enumerate() {
        // Some README blocks are illustrative snippets that show JUST
        // the `[[environments]]` table without a surrounding `[project]`
        // table. Prepend a minimal manifest preamble so those snippets
        // can be validated for SYNTAX without forcing every doc example
        // to be a full coral.toml document.
        let block = if raw_block.contains("[project]") {
            raw_block.clone()
        } else {
            format!(
                "apiVersion = \"coral.dev/v1\"\n\
                 [project]\n\
                 name = \"readme-test-fixture\"\n\
                 [[repos]]\n\
                 name = \"api\"\n\
                 url  = \"git@example.com/api.git\"\n\
                 [[repos]]\n\
                 name = \"db\"\n\
                 url  = \"git@example.com/db.git\"\n\
                 \n{raw_block}"
            )
        };
        let manifest = parse_toml(&block, Path::new("/tmp/coral.toml")).unwrap_or_else(|e| {
            panic!("README toml block #{block_idx} failed to parse:\n{e}\n--- block:\n{block}")
        });
        manifest.validate().unwrap_or_else(|e| {
            panic!("README toml block #{block_idx} failed to validate:\n{e}\n--- block:\n{block}")
        });
        for (env_idx, raw) in manifest.environments_raw.iter().enumerate() {
            let spec: EnvironmentSpec = raw.clone().try_into().unwrap_or_else(|e| {
                panic!(
                    "README toml block #{block_idx}, environments[{env_idx}] failed \
                     EnvironmentSpec deserialization:\n{e}\n--- block:\n{block}"
                )
            });
            assert!(
                !spec.services.is_empty(),
                "README toml block #{block_idx}, environments[{env_idx}]: \
                 services map is empty — likely an `[environments.<env>.services.*]` \
                 path mismatch. Use `[environments.services.*]` so the table attaches \
                 to the currently-open `[[environments]]` array entry. block:\n{block}"
            );
        }
    }
}

/// Pull every fenced ```toml block that declares `[[environments]]`
/// out of the README. Tolerates both `\`\`\`toml` and `\`\`\` toml`
/// fences (whitespace before the language tag is rare but legal).
fn extract_toml_blocks_with_environments(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let is_toml_fence = trimmed.starts_with("```toml") || trimmed == "```toml";
        if !is_toml_fence {
            continue;
        }
        let mut buf = String::new();
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with("```") {
                break;
            }
            buf.push_str(inner);
            buf.push('\n');
        }
        if buf.contains("[[environments]]") {
            out.push(buf);
        }
    }
    out
}
