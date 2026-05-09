//! `coral skill build` — produces an Anthropic-Skills-compatible zip
//! bundle at `dist/coral-skill-<version>.zip` from `template/`.
//!
//! ## What ships in the bundle
//!
//! - `template/agents/*.md` → `agents/*.md`
//! - `template/prompts/*.md` → `prompts/*.md`
//! - `template/hooks/*.sh`   → `hooks/*.sh`
//! - Auto-generated `SKILL.md` at the zip root with frontmatter
//!   (`name`, `description`, `version`).
//!
//! ## What is intentionally excluded
//!
//! - `template/schema/`     — Coral-specific lint schema; not portable.
//! - `template/workflows/`  — GitHub Actions workflow yaml; not part
//!   of the agent/skill surface.
//! - `template/commands/`   — slash-command wrappers tied to Claude
//!   Code's `commands/` convention; the agents themselves are the
//!   portable surface.
//!
//! ## Determinism
//!
//! Two consecutive `coral skill build` invocations produce
//! byte-identical zips. Achieved by:
//! 1. Sorting entry paths before writing.
//! 2. Pinning every entry's mtime to the zip-format minimum
//!    (`1980-01-01T00:00:00Z`) — DOS time has no notion of
//!    "current time", so without this every run would diff.
//! 3. Default deflate level (the same level on every machine).
//!
//! ## `coral skill publish`
//!
//! Stub for v0.22.6; deferred to v0.23+. Prints a one-line message
//! pointing the user at `coral skill build` + the
//! Anthropic-Skills repo, exits 0.

use anyhow::{Context, Result};
use coral_core::atomic::atomic_write_string;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use zip::CompressionMethod;
use zip::DateTime;
use zip::write::{SimpleFileOptions, ZipWriter};

/// One entry in the skill bundle: zip-internal path + raw bytes.
struct Entry {
    /// Path inside the zip (e.g. `agents/wiki-linter.md`,
    /// `SKILL.md`). Always forward-slash separated; never starts
    /// with `/`.
    zip_path: String,
    /// Raw file bytes. For text files (markdown, shell), this is
    /// UTF-8 with the on-disk line endings preserved.
    bytes: Vec<u8>,
}

/// Subdirectories under `template/` that ship in the bundle.
///
/// Order is irrelevant — entries get sorted by `zip_path` before
/// writing for determinism. Listed in `agents`, `prompts`, `hooks`
/// order to match the SKILL.md "Contents" section.
const INCLUDED_SUBDIRS: &[&str] = &["agents", "prompts", "hooks"];

/// Build the bundle.
///
/// `output` overrides the default `dist/coral-skill-<version>.zip`.
/// When the user passes `--output`, `dist/` is NOT created (the
/// caller picked a different location on purpose).
pub fn build(output: Option<PathBuf>) -> Result<ExitCode> {
    let template_dir = locate_template_dir()?;
    let entries = collect_entries(&template_dir)?;
    let zip_bytes = write_zip(&entries)?;

    let target = resolve_output_path(output.as_deref())?;
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
    }
    write_bytes_atomic(&target, &zip_bytes)?;

    let uncompressed: usize = entries.iter().map(|e| e.bytes.len()).sum();
    println!(
        "wrote {} ({} files, {} bytes uncompressed)",
        target.display(),
        entries.len(),
        uncompressed
    );
    Ok(ExitCode::SUCCESS)
}

/// `coral skill publish` stub.
///
/// Real implementation is deferred to v0.23+ pending the
/// Anthropic-Skills fork-and-PR flow. For now: print exactly the
/// message specified in §D5 and exit 0.
pub fn publish() -> Result<ExitCode> {
    println!(
        "publish is deferred to v0.23+; for now, run `coral skill build` \
         and submit the zip manually to https://github.com/anthropics/skills"
    );
    Ok(ExitCode::SUCCESS)
}

/// Resolve where `template/` lives relative to the workspace root.
///
/// Strategy: walk up from cwd looking for a `template/` sibling of
/// `Cargo.toml` whose `[workspace]` table mentions `crates/*`. This
/// matches how `coral` is run from the workspace root in dev and
/// keeps the integration tests stable when invoked from anywhere
/// under the workspace.
fn locate_template_dir() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("getting cwd")?;
    let mut cursor: Option<&Path> = Some(cwd.as_path());
    while let Some(p) = cursor {
        let template = p.join("template");
        let cargo = p.join("Cargo.toml");
        if template.is_dir() && cargo.is_file() {
            // Cheap sanity-check: the workspace Cargo.toml mentions
            // `crates/*` (i.e. this is the Coral workspace root).
            if let Ok(s) = std::fs::read_to_string(&cargo) {
                if s.contains("members") && s.contains("crates/") {
                    return Ok(template);
                }
            }
        }
        cursor = p.parent();
    }
    anyhow::bail!(
        "could not locate `template/` next to a workspace `Cargo.toml`; \
         run `coral skill build` from a Coral workspace checkout"
    )
}

/// Walk `template/{agents,prompts,hooks}` and gather every regular
/// file. Returns Entries sorted by `zip_path`.
///
/// SKILL.md is generated separately and prepended in `write_zip`.
fn collect_entries(template_dir: &Path) -> Result<Vec<Entry>> {
    let mut out: Vec<Entry> = Vec::new();
    for sub in INCLUDED_SUBDIRS {
        let dir = template_dir.join(sub);
        if !dir.is_dir() {
            // Tolerate a missing subdir — we'd rather build a
            // partial bundle than fail. The acceptance criteria
            // assume all three are present in `template/`, but
            // tests stub `template/` and we don't want to force
            // them to populate every subdir.
            continue;
        }
        let entries =
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("iterating {}", dir.display()))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .with_context(|| format!("non-UTF-8 filename in {}", dir.display()))?;
            let bytes =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            out.push(Entry {
                zip_path: format!("{sub}/{name}"),
                bytes,
            });
        }
    }
    out.sort_by(|a, b| a.zip_path.cmp(&b.zip_path));

    // SKILL.md is computed last because it summarizes the others
    // (their frontmatter `description` fields).
    let skill_md = generate_skill_md(&out);
    let mut all = Vec::with_capacity(out.len() + 1);
    all.push(Entry {
        zip_path: "SKILL.md".to_string(),
        bytes: skill_md.into_bytes(),
    });
    all.extend(out);
    // Re-sort with SKILL.md included so determinism holds regardless
    // of the order we pushed.
    all.sort_by(|a, b| a.zip_path.cmp(&b.zip_path));
    Ok(all)
}

/// Generate the auto-SKILL.md body.
///
/// Frontmatter `version` is `env!("CARGO_PKG_VERSION")` so it always
/// matches the running binary. Per-file descriptions are pulled
/// from each markdown's YAML frontmatter; missing/malformed →
/// fallback to empty string (we never fail the build over
/// frontmatter).
fn generate_skill_md(entries: &[Entry]) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str("name: coral\n");
    s.push_str("description: ");
    s.push_str(
        "Karpathy-style LLM Wiki maintainer for Git repos. Provides agents \
         (linter, validator, consolidator, bibliotecario, onboarder) and \
         prompt templates for wiki maintenance, lint auto-fix, consolidation, \
         and onboarding.\n",
    );
    s.push_str(&format!("version: {version}\n"));
    s.push_str("---\n");
    s.push_str("\n# Coral skill bundle\n\n");
    s.push_str(
        "Coral is a Rust CLI that maintains an LLM-friendly wiki sidecar \
         for Git repos. This skill bundle ships the agent personas and prompt \
         templates that drive Coral's wiki-maintenance flow, suitable for \
         direct use with Claude or any MCP-compatible agent.\n\n",
    );
    s.push_str("## Contents\n\n");

    for (heading, sub) in [
        ("Agents", "agents/"),
        ("Prompts", "prompts/"),
        ("Hooks", "hooks/"),
    ] {
        s.push_str(&format!("### {heading} (`{sub}`)\n\n"));
        let mut any = false;
        for e in entries {
            if !e.zip_path.starts_with(sub) {
                continue;
            }
            any = true;
            let filename = e.zip_path.strip_prefix(sub).unwrap_or(&e.zip_path);
            let desc = parse_description(&e.bytes).unwrap_or_default();
            s.push_str(&format!("- `{filename}`: {desc}\n"));
        }
        if !any {
            s.push_str("- (none)\n");
        }
        s.push('\n');
    }

    s.push_str("## Usage\n\n");
    s.push_str(&format!(
        "Install: `unzip coral-skill-{version}.zip -d ~/.claude/skills/coral`  \n"
    ));
    s.push_str(
        "Use: agents reference each other and the prompts via filename — \
         the skill is self-contained.\n\n",
    );
    s.push_str("## Source\n\n");
    s.push_str(&format!(
        "Coral v{version} — https://github.com/agustincbajo/Coral\n"
    ));
    s
}

/// Pull `description: ...` out of a YAML frontmatter block.
///
/// Returns `None` if the file has no frontmatter or no
/// `description` key. Tolerant of:
/// - Missing frontmatter (markdown without `---` fences).
/// - Multi-line `description` values aren't unfolded — we take the
///   first line only, which matches the Anthropic-Skills convention
///   and how Claude Code's existing agents/ files are written.
fn parse_description(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?;
    let mut lines = s.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            return None;
        }
        if let Some(rest) = line.strip_prefix("description:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Write entries into a deflate zip archive, returning the bytes.
///
/// Determinism levers:
/// - `last_modified_time(DateTime::default())` pins entries to
///   `1980-01-01T00:00:00Z`, the zip-format minimum. Without
///   this, the local file header's "last mod time/date" embeds
///   `now()` and every run produces a different SHA.
/// - `unix_permissions(0o644)` so the file mode bits in the
///   external attributes don't reflect umask drift.
/// - Default deflate compression level is the same on every
///   build of `flate2` we'll ever see; we don't override it.
fn write_zip(entries: &[Entry]) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zw = ZipWriter::new(cursor);
        let opts: SimpleFileOptions = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(DateTime::default())
            .unix_permissions(0o644);
        for e in entries {
            zw.start_file(&e.zip_path, opts)
                .with_context(|| format!("starting zip entry {}", e.zip_path))?;
            zw.write_all(&e.bytes)
                .with_context(|| format!("writing zip entry {}", e.zip_path))?;
        }
        zw.finish().context("finalizing zip archive")?;
    }
    Ok(buf)
}

/// Default output path = `dist/coral-skill-<version>.zip` relative
/// to the cwd. `--output` overrides verbatim.
fn resolve_output_path(override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    let version = env!("CARGO_PKG_VERSION");
    Ok(PathBuf::from("dist").join(format!("coral-skill-{version}.zip")))
}

/// Atomic write of a binary blob.
///
/// `coral_core::atomic::atomic_write_string` only takes `&str`, so
/// we reuse the same temp+rename pattern (sibling tempfile in the
/// same dir, then atomic rename). Borrowing the byte path via
/// `from_utf8` would round-trip text; for arbitrary bytes we go
/// raw via `tempfile::NamedTempFile` in the same directory.
///
/// Why not just `std::fs::write`: a partial run leaves a torn zip
/// on disk and the next read would error mid-archive. The
/// tempfile + persist pattern guarantees readers see either the
/// full previous zip (if any) or the full new one — never a torn
/// state.
fn write_bytes_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    // Hot path: when target is a UTF-8 string and `bytes` happens
    // to be UTF-8, defer to the existing helper to keep one
    // implementation. Zip bytes are emphatically NOT UTF-8 (they
    // start with `PK\x03\x04`), so this branch never fires for
    // our bundle, but it's cheap and makes the helper consistent.
    if let Ok(s) = std::str::from_utf8(bytes) {
        atomic_write_string(target, s).map_err(|e| anyhow::anyhow!(e))?;
        return Ok(());
    }
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;
    let mut tmp = tempfile::NamedTempFile::new_in(&parent)
        .with_context(|| format!("creating temp file in {}", parent.display()))?;
    tmp.write_all(bytes)
        .with_context(|| format!("writing temp file for {}", target.display()))?;
    tmp.flush()
        .with_context(|| format!("flushing temp file for {}", target.display()))?;
    tmp.persist(target)
        .with_context(|| format!("persisting temp file to {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_description_handles_simple_frontmatter() {
        let src = b"---\nname: foo\ndescription: A short blurb.\n---\n\nbody";
        assert_eq!(parse_description(src).as_deref(), Some("A short blurb."));
    }

    #[test]
    fn parse_description_returns_none_without_frontmatter() {
        let src = b"# Heading\n\ndescription: not in frontmatter\n";
        assert!(parse_description(src).is_none());
    }

    #[test]
    fn parse_description_returns_none_when_key_missing() {
        let src = b"---\nname: foo\n---\n\nbody";
        assert!(parse_description(src).is_none());
    }

    #[test]
    fn skill_md_includes_version_from_cargo() {
        let entries: Vec<Entry> = Vec::new();
        let md = generate_skill_md(&entries);
        assert!(
            md.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))),
            "SKILL.md frontmatter must carry the cargo version"
        );
    }
}
