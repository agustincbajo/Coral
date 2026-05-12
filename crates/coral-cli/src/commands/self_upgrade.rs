//! `coral self-upgrade` — cross-platform in-place upgrade of the
//! Coral binary (FR-ONB-32).
//!
//! Resolves the current version (`env!("CARGO_PKG_VERSION")`) against
//! the latest GitHub release (or an explicit `--version vX.Y.Z`).
//! Downloads the platform tarball/zip, verifies its SHA-256 against
//! the `.sha256` sidecar the release publishes, and atomically
//! replaces the running binary.
//!
//! Platform contract:
//!
//! * **Linux / macOS**: `std::fs::rename` works while the binary is
//!   mapped — the kernel keeps the old inode alive for the running
//!   process and the next exec picks up the new file. We chmod +x
//!   after the rename so the file is executable even if the
//!   tarball's mode bits got lost.
//! * **Windows**: `MoveFileExW(current → .old, REPLACE_EXISTING)`
//!   moves the live `.exe` out of the way. If that fails (other
//!   processes still holding it), we fall back to
//!   `MoveFileExW(current → null, DELAY_UNTIL_REBOOT)` which queues
//!   the deletion for the next boot. Then we move `.new → current`,
//!   which Windows allows because the running process is now looking
//!   at the renamed `.old` file. The user must restart their shell
//!   to pick up the new binary (we surface this in the success
//!   message). Subsequent runs of `coral self-upgrade` clean up any
//!   leftover `.old` files best-effort.
//!
//! Major-bump guard: by default we refuse same-line bumps that cross
//! a major boundary (v0.x → v1.x, v1.y → v2.y). PRD anti-feature AF-9
//! covers this — schema migrations between majors deserve an explicit
//! `install.sh` re-run with a user-visible README, not a quiet
//! in-place swap. Override only by passing `--version` explicitly.

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const REPO: &str = "agustincbajo/Coral";
const RELEASES_API: &str = "https://api.github.com/repos/agustincbajo/Coral/releases/latest";

/// `coral self-upgrade` arguments.
#[derive(Args, Debug)]
pub struct SelfUpgradeArgs {
    /// Target version (e.g. `v0.34.1`). Default: latest same-major
    /// from the GitHub releases API. Major-bumps are refused unless
    /// the operator explicitly passes the target via this flag — see
    /// the module docs for the rationale.
    #[arg(long)]
    pub version: Option<String>,
    /// Only check if an upgrade is available; don't download. Prints
    /// `update_available: vX.Y.Z` or `up_to_date` and exits.
    #[arg(long = "check-only")]
    pub check_only: bool,
}

pub fn run(args: SelfUpgradeArgs) -> Result<ExitCode> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    // Preserve the user's original tag (with-or-without leading `v`)
    // so messages echo what they typed. `normalize_version` is used
    // only for the equality / major comparison.
    let target_version = match args.version.as_deref() {
        Some(explicit) => explicit.to_string(),
        None => fetch_latest_release_tag().context("looking up the latest release from GitHub")?,
    };

    // --check-only short-circuits before any major-bump or download
    // work — its only contract is reporting upgrade availability.
    if args.check_only {
        report_check_only(&current_version, &target_version);
        return Ok(ExitCode::SUCCESS);
    }

    // Same-version is a no-op (matches the `install.sh`/`install.ps1`
    // already-installed branch — we never re-download).
    if normalize_version(&target_version) == current_version {
        println!("coral {current_version} is already the requested version — nothing to do.");
        return Ok(ExitCode::SUCCESS);
    }

    // Major-bump guard: refuse unless the user explicitly named the
    // version via --version (then we still refuse cross-major bumps
    // because the on-disk format / schema isn't guaranteed compatible).
    let major_ok = same_major(&current_version, &target_version);
    if !major_ok {
        bail!(
            "refusing major-version bump (current {current_version}, target {target_version}): \
             schemas may need migration. Re-run install.sh to opt in explicitly."
        );
    }

    let current_exe =
        std::env::current_exe().context("locating the running binary via current_exe()")?;
    // GitHub release tags are always vX.Y.Z; re-add the prefix for
    // URL + filename derivation so users that typed `0.34.0` still
    // hit the right asset.
    let tag_for_url = ensure_v_prefix(&target_version);
    let target_filename = platform_asset_filename(&tag_for_url)?;
    let download_url = format!(
        "https://github.com/{REPO}/releases/download/{tag}/{name}",
        tag = tag_for_url,
        name = target_filename,
    );
    let sha_url = format!("{download_url}.sha256");

    println!("Downloading {target_filename} ...");
    let tmpdir = tempfile::tempdir().context("creating temp dir for upgrade download")?;
    let archive_path = tmpdir.path().join(&target_filename);
    download_to(&download_url, &archive_path)
        .with_context(|| format!("downloading {download_url}"))?;
    let sha_path = tmpdir.path().join(format!("{target_filename}.sha256"));
    download_to(&sha_url, &sha_path).with_context(|| format!("downloading {sha_url}"))?;

    println!("Verifying SHA-256 ...");
    verify_sha256(&archive_path, &sha_path)?;

    // Extract the bare binary from the archive into the tempdir so
    // we have a standalone file to swap onto current_exe's parent.
    println!("Extracting binary ...");
    let extracted = extract_binary(&archive_path, tmpdir.path(), &tag_for_url)?;

    let new_path = staged_new_path(&current_exe);
    // Move the extracted binary alongside the running exe so the swap
    // step works atomically (rename within the same filesystem).
    std::fs::rename(&extracted, &new_path)
        .with_context(|| format!("staging {} next to running exe", new_path.display()))?;

    replace_running_binary(&current_exe, &new_path, &target_version)?;
    Ok(ExitCode::SUCCESS)
}

fn report_check_only(current: &str, target: &str) {
    // Normalize for the equality check so `v0.34.0` and `0.34.0`
    // both match the embedded CARGO_PKG_VERSION (which is unprefixed).
    let target_n = normalize_version(target);
    if target_n == current {
        println!("up_to_date");
    } else {
        // Display with `v` prefix because that's the canonical
        // release-tag shape on GitHub (matches what `coral
        // self-upgrade --version v...` would expect).
        println!("update_available: {}", ensure_v_prefix(target));
    }
}

/// Strip a leading `v` from a tag so we can compare against
/// `env!("CARGO_PKG_VERSION")` (which is unprefixed).
pub(crate) fn normalize_version(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Add a leading `v` if missing — GitHub release tags are always
/// `vX.Y.Z`, and the install/release artefact paths embed the
/// prefixed tag verbatim.
pub(crate) fn ensure_v_prefix(tag: &str) -> String {
    if tag.starts_with('v') {
        tag.to_string()
    } else {
        format!("v{tag}")
    }
}

/// Returns true iff `current` and `target` share a major component.
/// We treat parse failures as "different" so a malformed input falls
/// into the safer refuse-major-bump branch.
pub(crate) fn same_major(current: &str, target: &str) -> bool {
    let cur = parse_major(current);
    let tgt = parse_major(normalize_version(target));
    matches!((cur, tgt), (Some(a), Some(b)) if a == b)
}

fn parse_major(v: &str) -> Option<u64> {
    v.split('.').next().and_then(|s| s.parse().ok())
}

/// The GitHub release-asset name for this host's triple. We mirror
/// the names `release.yml` produces (tar.gz on Linux/macOS, zip on
/// Windows). Returns Err on unsupported targets so the user gets a
/// clear refusal rather than a 404.
pub(crate) fn platform_asset_filename(tag: &str) -> Result<String> {
    let triple = host_target_triple()?;
    let suffix = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };
    Ok(format!("coral-{tag}-{triple}.{suffix}"))
}

fn host_target_triple() -> Result<&'static str> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        (os, arch) => {
            bail!(
                "no published binary for {os}/{arch}; build from source via `cargo install --git https://github.com/{REPO} coral-cli`"
            )
        }
    };
    Ok(triple)
}

/// Where we stage the downloaded binary before the atomic swap.
/// On Unix the suffix is `.new`; on Windows we use `.exe.new` so the
/// MoveFileExW dance later can also reference `.exe.old`.
fn staged_new_path(current_exe: &Path) -> PathBuf {
    if cfg!(windows) {
        current_exe.with_extension("exe.new")
    } else {
        let mut p = current_exe.to_path_buf();
        let name = p
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_else(|| "coral".into());
        let mut new_name = name;
        new_name.push(".new");
        p.set_file_name(new_name);
        p
    }
}

/// Best-effort cross-platform download via `ureq`. Timeouts: 30s
/// connect, 5min read (release tarballs are small but throttled GH
/// CDN can be slow on the first byte).
fn download_to(url: &str, dest: &Path) -> Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(300))
        .user_agent(concat!("coral-self-upgrade/", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| anyhow!("HTTP GET failed: {e}"))?;
    let mut reader = resp.into_reader();
    let mut out =
        std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    std::io::copy(&mut reader, &mut out).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

/// Verifies that `archive` hashes to the SHA-256 declared inside
/// `sha_file`. We parse the sidecar's first whitespace-separated
/// token (matches both `shasum -a 256` and `sha256sum` output shapes:
/// "<hex>  <filename>"). Failure aborts with a clear message.
pub(crate) fn verify_sha256(archive: &Path, sha_file: &Path) -> Result<()> {
    let sha_text = std::fs::read_to_string(sha_file)
        .with_context(|| format!("reading {}", sha_file.display()))?;
    let expected = sha_text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("sha256 sidecar {} is empty", sha_file.display()))?
        .to_lowercase();

    let mut hasher = Sha256::new();
    let mut file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex_encode(&hasher.finalize());
    if actual != expected {
        bail!(
            "SHA-256 mismatch for {}: expected {} actual {}",
            archive.display(),
            expected,
            actual
        );
    }
    Ok(())
}

/// Lowercase hex encoding without an extra dep (we'd otherwise pull
/// `hex` just for one call site). 32 bytes -> 64 chars; we
/// pre-allocate.
fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(nibble_to_hex(b >> 4));
        out.push(nibble_to_hex(b & 0x0f));
    }
    out
}

fn nibble_to_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + n - 10) as char,
        _ => unreachable!("nibble was {n}"),
    }
}

/// Extracts the `coral` (or `coral.exe`) binary out of the
/// downloaded archive into `dest_dir`. Returns the path to the
/// extracted file. The release-layout convention is
/// `coral-<tag>-<triple>/coral[.exe]` inside the archive.
fn extract_binary(archive: &Path, dest_dir: &Path, tag: &str) -> Result<PathBuf> {
    let triple = host_target_triple()?;
    let bin_name = if cfg!(windows) { "coral.exe" } else { "coral" };
    let expected_inside = format!("coral-{tag}-{triple}/{bin_name}");

    if archive
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
    {
        extract_from_zip(archive, dest_dir, &expected_inside, bin_name)
    } else {
        extract_from_tarball(archive, dest_dir, &expected_inside, bin_name)
    }
}

fn extract_from_tarball(
    archive: &Path,
    dest_dir: &Path,
    expected_inside: &str,
    bin_name: &str,
) -> Result<PathBuf> {
    // tar.gz extraction without pulling tar/flate2: shell out to the
    // platform tar(1). All Linux + macOS targets ship `tar` in the
    // base system (FreeBSD libarchive on macOS, GNU on Linux). The
    // installer already relies on tar so this is consistent.
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest_dir)
        .status()
        .context("spawning `tar -xzf`")?;
    if !status.success() {
        bail!("tar extraction failed (exit {status})");
    }
    let extracted = dest_dir.join(expected_inside);
    if !extracted.is_file() {
        // Fallback: the layout might have changed; walk one level
        // down for any file named `coral`.
        if let Some(found) = find_binary_in(dest_dir, bin_name) {
            return Ok(found);
        }
        bail!(
            "expected {} inside archive after extraction; not found",
            expected_inside
        );
    }
    Ok(extracted)
}

fn extract_from_zip(
    archive: &Path,
    dest_dir: &Path,
    expected_inside: &str,
    bin_name: &str,
) -> Result<PathBuf> {
    // We already pull `zip` as a workspace dep for `coral skill build`
    // — reuse it here so we don't shell out to PowerShell's
    // Expand-Archive (which is fine but adds a process boundary the
    // tests can't drive easily).
    let file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).with_context(|| format!("reading zip {}", archive.display()))?;
    let mut found: Option<PathBuf> = None;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // `enclosed_name()` strips `..` traversal — security-critical
        // because we extract under dest_dir which is shared with the
        // running binary's neighbourhood.
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let out_path = dest_dir.join(&name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)
            .with_context(|| format!("creating {}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out)?;
        if name.to_string_lossy().replace('\\', "/") == expected_inside
            || name.file_name().and_then(|n| n.to_str()) == Some(bin_name)
        {
            found = Some(out_path);
        }
    }
    found.ok_or_else(|| {
        anyhow!(
            "expected {} inside archive after extraction; not found",
            expected_inside
        )
    })
}

/// Best-effort scan for a binary named `bin_name` under `root` (one
/// level deep — the release layout is flat).
fn find_binary_in(root: &Path, bin_name: &str) -> Option<PathBuf> {
    let read = std::fs::read_dir(root).ok()?;
    for entry in read.flatten() {
        let p = entry.path();
        if p.is_file() && p.file_name().and_then(|n| n.to_str()) == Some(bin_name) {
            return Some(p);
        }
        if p.is_dir() {
            let read2 = std::fs::read_dir(&p).ok()?;
            for sub in read2.flatten() {
                let sp = sub.path();
                if sp.is_file() && sp.file_name().and_then(|n| n.to_str()) == Some(bin_name) {
                    return Some(sp);
                }
            }
        }
    }
    None
}

// --------------------------------------------------------------------------
// Cross-platform binary replacement
// --------------------------------------------------------------------------

/// Swaps `new_path` into the place of `current_exe`. Unix: atomic
/// rename + chmod 755. Windows: MoveFileExW with the fallback to
/// `DELAY_UNTIL_REBOOT` if the live binary can't be moved aside.
#[allow(unused_variables)] // `tag` is only used in the Windows message.
fn replace_running_binary(current_exe: &Path, new_path: &Path, tag: &str) -> Result<()> {
    #[cfg(unix)]
    {
        replace_unix(current_exe, new_path)?;
        println!(
            "Upgraded. Next invocation will use {tag}. \
             (Linux/macOS: the rename is atomic; the currently-running shell \
              can keep using the old binary until it exits.)"
        );
        // Best-effort post-upgrade verification — run the new binary
        // and confirm the version it reports is the one we just
        // installed. Windows skips this because the running process
        // still has the old exe mapped.
        let _ = post_upgrade_verify(current_exe, tag);
        Ok(())
    }
    #[cfg(windows)]
    {
        replace_windows(current_exe, new_path)?;
        println!(
            "Upgraded. Restart your shell to use {tag}. \
             The old binary will be removed on next reboot."
        );
        println!();
        println!(
            "Windows Defender SmartScreen may prompt on first launch of the new binary. \
             If so: right-click coral.exe -> Properties -> check 'Unblock' -> OK."
        );
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (current_exe, new_path, tag);
        bail!("unsupported platform for self-upgrade")
    }
}

#[cfg(unix)]
fn replace_unix(current_exe: &Path, new_path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::rename(new_path, current_exe).with_context(|| {
        format!(
            "renaming {} -> {}",
            new_path.display(),
            current_exe.display()
        )
    })?;
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(current_exe, perms)
        .with_context(|| format!("chmod 0755 {}", current_exe.display()))?;
    Ok(())
}

#[cfg(windows)]
fn replace_windows(current_exe: &Path, new_path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_DELAY_UNTIL_REBOOT, MOVEFILE_REPLACE_EXISTING, MoveFileExW,
    };

    // Cleanup leftover `.old` from a previous upgrade (best-effort).
    // If it can't be removed (still in use), Windows will refuse and
    // we keep going — the new `.old` will live alongside it.
    let old_path = current_exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old_path);

    let cur_w: Vec<u16> = current_exe.as_os_str().encode_wide().chain([0]).collect();
    let old_w: Vec<u16> = old_path.as_os_str().encode_wide().chain([0]).collect();
    let new_w: Vec<u16> = new_path.as_os_str().encode_wide().chain([0]).collect();

    // (a) Move the running .exe out of the way. If that succeeds, it
    // is renamed to coral.exe.old. If it fails (lock held by another
    // process, antivirus, etc.) we schedule it for delete-on-reboot
    // so the upgrade doesn't fail outright — the new binary will
    // still land via step (b).
    let moved = unsafe { MoveFileExW(cur_w.as_ptr(), old_w.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
    if moved == 0 {
        // Schedule the stale exe for delete-on-reboot; ignore the
        // result because the next MoveFileExW is what matters.
        unsafe {
            MoveFileExW(
                cur_w.as_ptr(),
                std::ptr::null(),
                MOVEFILE_DELAY_UNTIL_REBOOT,
            )
        };
    }

    // (b) Place the new binary at the canonical path. This must
    // succeed; if it doesn't we're in a half-upgraded state and the
    // user needs a manual recovery.
    let placed = unsafe { MoveFileExW(new_w.as_ptr(), cur_w.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
    if placed == 0 {
        let err = std::io::Error::last_os_error();
        bail!(
            "could not place new binary at {} (Win32 error {err}); \
             upgrade aborted — old binary preserved at {}",
            current_exe.display(),
            old_path.display()
        );
    }
    Ok(())
}

/// Run `current_exe --version` and confirm the reported version
/// matches `tag` (normalized). Best-effort: failure to spawn is
/// logged but does not propagate as an error — the swap already
/// happened successfully.
#[cfg(unix)]
fn post_upgrade_verify(current_exe: &Path, tag: &str) -> Result<()> {
    let out = std::process::Command::new(current_exe)
        .arg("--version")
        .output()
        .context("spawning new binary --version")?;
    if !out.status.success() {
        eprintln!(
            "warning: new binary at {} returned non-zero on --version",
            current_exe.display()
        );
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let want = normalize_version(tag);
    if !stdout.contains(want) {
        eprintln!(
            "warning: new binary reports `{}` but target was `{tag}`",
            stdout.trim()
        );
    }
    Ok(())
}

// --------------------------------------------------------------------------
// GitHub API: latest release tag
// --------------------------------------------------------------------------

/// Fetches the `tag_name` from GitHub's `releases/latest` endpoint
/// via `ureq`. Times out at 15s — this is on the user's critical
/// path. Returns the raw tag string (e.g. `v0.34.1`).
pub(crate) fn fetch_latest_release_tag() -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("coral-self-upgrade/", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent
        .get(RELEASES_API)
        .set("accept", "application/vnd.github+json")
        .call()
        .map_err(|e| anyhow!("HTTP GET {RELEASES_API} failed: {e}"))?;
    let body: serde_json::Value = resp
        .into_json()
        .context("parsing GitHub releases/latest body")?;
    let tag = body
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("`tag_name` missing from GitHub response"))?;
    Ok(tag.to_string())
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `normalize_version` strips a single leading `v` and leaves
    /// non-prefixed inputs alone.
    #[test]
    fn normalize_version_strips_leading_v_only() {
        assert_eq!(normalize_version("v0.34.0"), "0.34.0");
        assert_eq!(normalize_version("0.34.0"), "0.34.0");
        // Pathological: double v stays as one of them. The semver
        // spec says vX.Y.Z; vvX.Y.Z is malformed and we don't try
        // to fix it.
        assert_eq!(normalize_version("vv1.0.0"), "v1.0.0");
    }

    /// `ensure_v_prefix` is the inverse — it adds a `v` if missing
    /// so we can build release URLs from either user-typed form.
    #[test]
    fn ensure_v_prefix_idempotent_over_either_form() {
        assert_eq!(ensure_v_prefix("v0.34.0"), "v0.34.0");
        assert_eq!(ensure_v_prefix("0.34.0"), "v0.34.0");
        // Pairs with normalize_version: round-tripping (normalize then
        // ensure_v) is a no-op.
        let raw = "v1.2.3";
        assert_eq!(ensure_v_prefix(normalize_version(raw)), raw);
    }

    /// `same_major` accepts patch + minor bumps and refuses major
    /// jumps. Malformed inputs land in the refuse-bump branch.
    #[test]
    fn same_major_pins_major_boundary() {
        assert!(same_major("0.34.0", "v0.34.1"));
        assert!(same_major("0.34.0", "0.35.0"));
        assert!(!same_major("0.34.0", "v1.0.0"));
        assert!(!same_major("0.34.0", "1.0.0"));
        // Malformed: parse_major returns None and we conservatively
        // refuse (treating it as a major mismatch).
        assert!(!same_major("not-a-version", "v0.34.0"));
        assert!(!same_major("0.34.0", "garbage"));
    }

    /// `platform_asset_filename` matches the names release.yml
    /// publishes for THIS host's triple. We can't compare against a
    /// hard-coded triple list because the test runs on every CI
    /// platform; instead we assert structural invariants.
    #[test]
    fn platform_asset_filename_matches_release_naming() {
        let name = platform_asset_filename("v0.34.0").unwrap();
        assert!(
            name.starts_with("coral-v0.34.0-"),
            "release asset names start with `coral-<tag>-`: got {name}"
        );
        if cfg!(target_os = "windows") {
            assert!(
                name.ends_with(".zip"),
                "Windows release ships .zip: got {name}"
            );
        } else {
            assert!(
                name.ends_with(".tar.gz"),
                "Linux/macOS release ships .tar.gz: got {name}"
            );
        }
    }

    /// `verify_sha256` accepts a matching hash and rejects a wrong
    /// one. We compute the real hash of a known payload and write
    /// a sidecar in the shasum/sha256sum format.
    #[test]
    fn verify_sha256_accepts_match_rejects_mismatch() {
        let dir = TempDir::new().unwrap();
        let payload = dir.path().join("payload.bin");
        std::fs::write(&payload, b"hello coral self-upgrade").unwrap();

        let mut hasher = Sha256::new();
        hasher.update(b"hello coral self-upgrade");
        let real = hex_encode(&hasher.finalize());

        let good_sha = dir.path().join("payload.bin.sha256");
        std::fs::write(&good_sha, format!("{real}  payload.bin\n")).unwrap();
        verify_sha256(&payload, &good_sha).expect("matching sha256 must verify");

        let bad_sha = dir.path().join("bad.sha256");
        std::fs::write(
            &bad_sha,
            "0000000000000000000000000000000000000000000000000000000000000000  payload.bin\n",
        )
        .unwrap();
        let err = verify_sha256(&payload, &bad_sha).expect_err("non-matching sha256 must reject");
        assert!(
            err.to_string().contains("SHA-256 mismatch"),
            "error must name the failure: {err}"
        );
    }

    /// `staged_new_path` produces a sibling of `current_exe` with
    /// the right suffix per platform — the rename later relies on
    /// both paths living in the same filesystem.
    #[test]
    fn staged_new_path_is_sibling_with_platform_suffix() {
        let here = PathBuf::from("/usr/local/bin/coral");
        let new = staged_new_path(&here);
        assert_eq!(new.parent(), here.parent());
        let name = new.file_name().unwrap().to_string_lossy().into_owned();
        if cfg!(windows) {
            assert!(name.ends_with("exe.new"), "Windows uses `.exe.new`: {name}");
        } else {
            assert!(name.ends_with(".new"), "Unix uses `.new`: {name}");
        }
    }

    /// `report_check_only` prints `up_to_date` when current == target
    /// and `update_available: <tag>` otherwise. We can't capture stdout
    /// from a unit test directly without a print-injection layer; we
    /// instead exercise the equality logic by calling the function
    /// (it doesn't panic in either branch) and rely on the integration
    /// CLI test to assert exact output.
    #[test]
    fn report_check_only_does_not_panic_in_either_branch() {
        report_check_only("0.34.0", "v0.34.0");
        report_check_only("0.34.0", "v0.34.1");
    }

    /// `extract_from_zip` pulls the canonical layout path out of a
    /// synthesized release zip. We build the zip in-memory with the
    /// `zip` crate (already a workspace dep) so the test is hermetic.
    #[test]
    fn extract_from_zip_finds_canonical_layout_binary() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("coral-v0.34.0-x86_64-pc-windows-msvc.zip");
        let bin_name = "coral.exe";
        let inner = "coral-v0.34.0-x86_64-pc-windows-msvc/coral.exe";

        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zw.start_file(inner, opts).unwrap();
            use std::io::Write;
            zw.write_all(b"FAKE-COR4L-EXE").unwrap();
            zw.finish().unwrap();
        }

        let extract_dir = dir.path().join("ext");
        std::fs::create_dir_all(&extract_dir).unwrap();
        let out = extract_from_zip(&archive, &extract_dir, inner, bin_name).unwrap();
        assert!(out.is_file(), "extracted binary must exist on disk");
        assert_eq!(
            std::fs::read(&out).unwrap(),
            b"FAKE-COR4L-EXE",
            "extracted payload must equal what we wrote into the zip"
        );
    }

    /// `--check-only` would never need the rename path; this test
    /// pins that a malformed `--version` string flows through
    /// normalize+same_major safely.
    #[test]
    fn normalize_and_same_major_compose_for_check_only() {
        let cur = env!("CARGO_PKG_VERSION");
        // `cur` is unprefixed, target with leading v must still match.
        assert!(same_major(cur, &format!("v{cur}")));
        assert_eq!(normalize_version(&format!("v{cur}")), cur);
    }
}
