//! Progress reporting and observability helpers.
//!
//! v0.41 P1: lightweight `progress!` macro for human-readable runner
//! feedback on stderr. Zero external dependencies — uses `eprintln!`.
//!
//! # Gate logic
//!
//! Output is suppressed when:
//! - `CORAL_QUIET=1` is set (explicit opt-out), OR
//! - `--quiet` was set at the CLI level (callers set `CORAL_QUIET=1`
//!   before invoking `progress!`), OR
//! - stderr is not a TTY **and** `CORAL_VERBOSE` is not set
//!   (piped/CI mode: assume output is machine-read and suppress noisy
//!   human-facing progress lines).
//!
//! Override table:
//!
//! | CORAL_QUIET | CORAL_VERBOSE | stderr TTY | Output? |
//! |-------------|---------------|------------|---------|
//! | 1           | any           | any        | no      |
//! | 0/unset     | 1             | any        | yes     |
//! | 0/unset     | 0/unset       | yes        | yes     |
//! | 0/unset     | 0/unset       | no         | no      |
//!
//! # Macro forms
//!
//! ```text
//! // Step in progress — ▶ message  (key: val, ...)
//! progress!(step, "message"; key = val, ...);
//!
//! // Step completed — ✓ message  (key: val, ...)
//! progress!(done, step, "message"; key = val, ...);
//!
//! // Step failed   — ✗ message  (key: val, ...)
//! progress!(fail, step, "message"; key = val, ...);
//! ```
//!
//! The `step` / `done` / `fail` token controls the Unicode prefix
//! character. The `;` + `key = val` pairs are optional — you may omit
//! the whole suffix.

use std::io::IsTerminal as _;

/// Returns `true` when progress output should be emitted to stderr.
///
/// Called by the [`progress!`] macro before every print so that the
/// gate check happens at each call-site rather than once at startup —
/// env vars set after `main()` (e.g. test helpers) are picked up
/// correctly.
pub fn progress_enabled() -> bool {
    // Explicit quiet → always suppress.
    if std::env::var("CORAL_QUIET").as_deref() == Ok("1") {
        return false;
    }
    // Explicit verbose → always emit.
    if std::env::var("CORAL_VERBOSE").as_deref() == Ok("1") {
        return true;
    }
    // Otherwise: only emit when stderr is a real terminal.
    std::io::stderr().is_terminal()
}

/// Truncate `s` to at most `max_chars` characters, appending `…` when
/// truncated. Used by the verbose prompt/response dump path.
pub fn truncate_for_display(s: &str, max_chars: usize) -> String {
    if max_chars == 0 || s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

/// Emit a `▶ message  (kvs)` line to stderr when progress is enabled.
///
/// This is the low-level emitter called by the [`progress!`] macro.
/// Prefer the macro over calling this directly.
///
/// - `prefix`: the Unicode glyph (▶ / ✓ / ✗).
/// - `message`: the human-readable step description.
/// - `kvs`: key-value annotation pairs appended in `(k: v, ...)` form.
///   Pass an empty slice to omit the parenthetical.
pub fn emit_progress(prefix: &str, message: &str, kvs: &[(&str, &dyn std::fmt::Display)]) {
    if !progress_enabled() {
        return;
    }
    if kvs.is_empty() {
        eprintln!("{prefix} {message}");
    } else {
        let pairs: Vec<String> = kvs.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        eprintln!("{prefix} {message}  ({})", pairs.join(", "));
    }
}

/// Three-form progress macro for runner observability.
///
/// # Forms
///
/// ```rust,ignore
/// // No annotations.
/// progress!(step, "Calling claude CLI...");
///
/// // With key=value annotations.
/// progress!(step, "Calling claude CLI..."; model = "sonnet");
///
/// // Done (✓) form — first arg after `done` is reserved for future
/// // "step name" grouping but currently ignored in the rendered output.
/// progress!(done, "bootstrap", "Got response"; tokens = 1234);
///
/// // Fail (✗) form.
/// progress!(fail, "bootstrap", "Runner returned non-zero exit"; code = 1);
/// ```
///
/// All forms respect [`progress_enabled`] — when suppressed, this is
/// a zero-cost no-op (the condition is checked inside [`emit_progress`]
/// which all forms delegate to).
#[macro_export]
macro_rules! progress {
    // step — no key=value pairs
    (step, $msg:expr) => {
        $crate::observability::emit_progress("▶", $msg, &[]);
    };
    // step — with key=value pairs
    (step, $msg:expr; $($key:ident = $val:expr),+ $(,)?) => {
        $crate::observability::emit_progress(
            "▶",
            $msg,
            &[$( (stringify!($key), &$val as &dyn ::std::fmt::Display) ),+],
        );
    };
    // done — no key=value pairs (step label is silently consumed)
    (done, $step:expr, $msg:expr) => {
        $crate::observability::emit_progress("✓", $msg, &[]);
    };
    // done — with key=value pairs
    (done, $step:expr, $msg:expr; $($key:ident = $val:expr),+ $(,)?) => {
        $crate::observability::emit_progress(
            "✓",
            $msg,
            &[$( (stringify!($key), &$val as &dyn ::std::fmt::Display) ),+],
        );
    };
    // fail — no key=value pairs
    (fail, $step:expr, $msg:expr) => {
        $crate::observability::emit_progress("✗", $msg, &[]);
    };
    // fail — with key=value pairs
    (fail, $step:expr, $msg:expr; $($key:ident = $val:expr),+ $(,)?) => {
        $crate::observability::emit_progress(
            "✗",
            $msg,
            &[$( (stringify!($key), &$val as &dyn ::std::fmt::Display) ),+],
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-touching tests to prevent cross-thread interference.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // ------------------------------------------------------------------ //
    // progress_enabled gate logic                                          //
    // ------------------------------------------------------------------ //

    /// Guard that saves + restores env vars so tests don't bleed state.
    /// Must be used under ENV_LOCK to prevent parallel test interference.
    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prior = std::env::var(key).ok();
            // SAFETY: caller holds ENV_LOCK, so no parallel env mutation.
            unsafe { std::env::set_var(key, val) };
            Self { key, prior }
        }

        fn unset(key: &'static str) -> Self {
            let prior = std::env::var(key).ok();
            unsafe { std::env::remove_var(key) };
            Self { key, prior }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(v) => unsafe { std::env::set_var(self.key, v) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    #[test]
    fn progress_disabled_when_coral_quiet_1() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        let _verbose = EnvGuard::unset("CORAL_VERBOSE");
        // Regardless of TTY state, CORAL_QUIET=1 wins.
        assert!(!progress_enabled(), "CORAL_QUIET=1 must suppress output");
    }

    #[test]
    fn progress_enabled_when_coral_verbose_1_and_quiet_unset() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::unset("CORAL_QUIET");
        let _verbose = EnvGuard::set("CORAL_VERBOSE", "1");
        assert!(progress_enabled(), "CORAL_VERBOSE=1 must enable output");
    }

    #[test]
    fn progress_quiet_overrides_verbose() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        let _verbose = EnvGuard::set("CORAL_VERBOSE", "1");
        assert!(
            !progress_enabled(),
            "CORAL_QUIET=1 must win even when CORAL_VERBOSE=1"
        );
    }

    // ------------------------------------------------------------------ //
    // truncate_for_display                                                 //
    // ------------------------------------------------------------------ //

    #[test]
    fn truncate_short_string_unchanged() {
        let s = "hello";
        assert_eq!(truncate_for_display(s, 100), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        let s = "hello";
        assert_eq!(truncate_for_display(s, 5), "hello");
    }

    #[test]
    fn truncate_long_string_gets_ellipsis() {
        let s = "hello world";
        let out = truncate_for_display(s, 5);
        assert_eq!(out, "hello…");
    }

    #[test]
    fn truncate_zero_max_returns_original() {
        // max_chars == 0 is treated as "no limit"
        let s = "hello";
        assert_eq!(truncate_for_display(s, 0), "hello");
    }

    #[test]
    fn truncate_multibyte_chars() {
        // "héllo" — 5 chars, each 1+ bytes
        let s = "héllo world";
        let out = truncate_for_display(s, 5);
        assert_eq!(out, "héllo…");
    }

    // ------------------------------------------------------------------ //
    // emit_progress smoke (visible only when stderr is a TTY)             //
    // We can't inspect eprintln! output in unit tests easily, so we       //
    // just verify the function doesn't panic under various inputs.        //
    // ------------------------------------------------------------------ //

    #[test]
    fn emit_progress_no_kvs_no_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        // Force quiet so tests don't spray to stderr in CI.
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        emit_progress("▶", "test message", &[]);
    }

    #[test]
    fn emit_progress_with_kvs_no_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        let tokens: u64 = 42;
        emit_progress("✓", "done", &[("tokens", &tokens)]);
    }

    // ------------------------------------------------------------------ //
    // progress! macro smoke                                                //
    // ------------------------------------------------------------------ //

    #[test]
    fn macro_step_form_no_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        progress!(step, "Starting step");
        progress!(step, "Starting step"; model = "sonnet");
    }

    #[test]
    fn macro_done_form_no_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        progress!(done, "run", "Finished");
        let tok: u64 = 100;
        progress!(done, "run", "Finished"; tokens = tok);
    }

    #[test]
    fn macro_fail_form_no_panic() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _quiet = EnvGuard::set("CORAL_QUIET", "1");
        progress!(fail, "run", "Something broke");
        let code: i32 = 1;
        progress!(fail, "run", "Something broke"; exit_code = code);
    }
}
