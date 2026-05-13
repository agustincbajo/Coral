//! v0.35 Phase C (P-H1) — pre-compress the embedded SPA bundle at
//! build time so the static-asset handler can serve `.gz` / `.br`
//! siblings when the client advertises support.
//!
//! Why pre-compress here instead of at request time?
//! 1. **CPU floor.** Brotli at quality 11 is ~10x slower than gzip and
//!    nowhere close to free at gzip level 9 either; doing it once per
//!    cargo build (and skipping when up-to-date) costs nothing per
//!    request, beating any runtime cache.
//! 2. **Same binary, smaller wire.** `include_dir!` bakes the siblings
//!    into the binary alongside the originals. End-users get a
//!    single-file `coral` with all three encodings available.
//! 3. **Deterministic.** Same input -> same compressed bytes. SLSA
//!    provenance + the deny check don't care about runtime entropy.
//!
//! The `.gz` / `.br` siblings are only generated for content the SPA
//! actually serves on the critical path: `.js`, `.css`, `.html`, `.svg`,
//! `.json`. Tiny files (< 1 KiB) skip compression — the headers cost
//! more than the saved bytes.
//!
//! `index.html` is excluded because the static-asset handler injects
//! the runtime config blob at request time; pre-compressing the literal
//! file would force a runtime re-compress after injection (defeating
//! the win) or leak the placeholder bytes to the client (defeating
//! the feature). The handler serves `index.html` uncompressed.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const COMPRESS_EXTENSIONS: &[&str] = &["js", "css", "svg", "json"];
const MIN_BYTES_TO_COMPRESS: u64 = 1024;
const INDEX_HTML: &str = "index.html";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist_dir = manifest_dir.join("assets").join("dist");

    // The dist dir may not exist on a fresh clone (the JS build step
    // populates it). Bail without error so `cargo check` on a clean
    // worktree still succeeds.
    if !dist_dir.is_dir() {
        println!("cargo:warning=coral-ui/assets/dist not found — skipping pre-compression");
        return;
    }

    // Tell cargo to re-run when the source bundle changes. We track
    // the dist dir as a whole; cargo invalidates on any descendant
    // mtime tick.
    println!("cargo:rerun-if-changed={}", dist_dir.display());

    walk_and_compress(&dist_dir);
}

fn walk_and_compress(root: &Path) {
    let entries = match fs::read_dir(root) {
        Ok(it) => it,
        Err(e) => {
            println!("cargo:warning=read_dir({}): {e}", root.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_and_compress(&path);
            continue;
        }
        if let Err(e) = maybe_compress(&path) {
            println!("cargo:warning=compress({}): {e}", path.display());
        }
    }
}

fn maybe_compress(path: &Path) -> std::io::Result<()> {
    // Skip the siblings we generated on a previous build.
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ext == "gz" || ext == "br" {
        return Ok(());
    }
    // Whitelist source extensions.
    if !COMPRESS_EXTENSIONS.contains(&ext.as_str()) {
        return Ok(());
    }
    // Skip `index.html` even if a future change makes it match the
    // whitelist — runtime config injection requires the raw bytes.
    if path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|n| n.eq_ignore_ascii_case(INDEX_HTML))
        .unwrap_or(false)
    {
        return Ok(());
    }

    let meta = fs::metadata(path)?;
    if meta.len() < MIN_BYTES_TO_COMPRESS {
        return Ok(());
    }

    let source = fs::read(path)?;
    let source_mtime = meta.modified()?;

    // Sibling paths.
    let gz_path = sibling_with_suffix(path, "gz");
    let br_path = sibling_with_suffix(path, "br");

    // Up-to-date check: skip the work when the sibling is newer than
    // the source. Saves ~seconds on incremental builds.
    if !needs_rebuild(&gz_path, source_mtime) {
        // gz fresh — but we still might need brotli.
    } else {
        write_gzip(&gz_path, &source)?;
    }
    if !needs_rebuild(&br_path, source_mtime) {
        // brotli fresh.
    } else {
        write_brotli(&br_path, &source)?;
    }
    Ok(())
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".");
    s.push(suffix);
    PathBuf::from(s)
}

fn needs_rebuild(sibling: &Path, source_mtime: std::time::SystemTime) -> bool {
    let Ok(meta) = fs::metadata(sibling) else {
        return true;
    };
    let Ok(sibling_mtime) = meta.modified() else {
        return true;
    };
    sibling_mtime < source_mtime
}

fn write_gzip(path: &Path, source: &[u8]) -> std::io::Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    let mut enc = GzEncoder::new(Vec::with_capacity(source.len() / 2), Compression::best());
    enc.write_all(source)?;
    let bytes = enc.finish()?;
    write_atomic(path, &bytes)
}

fn write_brotli(path: &Path, source: &[u8]) -> std::io::Result<()> {
    // Quality 11 is the brotli "max" setting; build-time cost is fine
    // because we run once per `cargo build` (and the cache key skips
    // when up-to-date).
    let mut out = Vec::with_capacity(source.len() / 2);
    {
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        writer.write_all(source)?;
        writer.flush()?;
    }
    write_atomic(path, &out)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Write to a temp sibling and rename — atomic on every platform we
    // care about. Defends against half-written files when the build is
    // killed mid-write.
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|s| s.to_str()).unwrap_or("bin")
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
