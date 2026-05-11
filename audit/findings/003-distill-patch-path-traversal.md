---
title: "security: `coral session distill` patch validator only inspects first `---`/`+++` header pair; multi-file diff bypasses slug check and `git apply --unsafe-paths` writes outside `.wiki/`"
severity: High
labels: security, distill, session
confidence: 4
cross_validated_by: [security-audit-agent, direct-code-read]
---

## Summary

`coral session distill` uses an LLM to propose unified-diff patches
against `.wiki/<slug>.md` pages. Before applying, each patch is checked
to ensure it targets the slug it claims to. That check
(`crates/coral-session/src/distill_patch.rs:217-241`) reads the diff
line-by-line and stops at the **first** `--- ` / `+++ ` header pair:

```rust
fn diff_targets_slug(diff: &str, target_slug: &str) -> bool {
    let mut minus_path: Option<String> = None;
    let mut plus_path: Option<String> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- ")
            && minus_path.is_none()                       // <-- only first
        {
            minus_path = Some(strip_diff_prefix(rest.trim()).to_string());
        } else if let Some(rest) = line.strip_prefix("+++ ")
            && plus_path.is_none()
        {
            plus_path = Some(strip_diff_prefix(rest.trim()).to_string());
            // After we've seen both headers, no need to keep parsing.
            break;                                        // <-- stops here
        }
    }
    …
}
```

The patch is then applied via `git apply --unsafe-paths --directory=.wiki`
(`distill_patch.rs:504`), where `--unsafe-paths` is documented in
`git-apply(1)` as: "By default, a patch that affects outside the
working area … is rejected as a mistake. This flag disables the check."
A unified diff can legally contain multiple file-pair headers. Only the
first is validated; the rest are applied as-is.

The module-level safety claim
(`distill_patch.rs:21-23`) reads:

```
Validation rationale for `git apply --unsafe-paths`: the flag
permits patches with paths outside the index, NOT untrusted paths
in any meaningful sense. The actual safety property comes from
[diff_targets_slug + slug allowlist on target_slug].
```

That safety property is incomplete: only the first pair is checked.

## Threat model

The distill subagent is an LLM. Its output is, by design, untrusted
input even when running locally. The README treats `prompt-injection`
as a first-class concern (lint check `injection-suspected` default-on
since v0.20.2). A user wiki page or chat transcript that an attacker
controls (e.g., an ingested third-party doc) can coax the LLM into
emitting a two-pair diff: first pair benign and matching the
`target_slug`, second pair escaping the wiki root.

## Repro (sketch — not executed in this audit)

1. Trick the distill runner into emitting a patch like:

   ```
   --- a/modules/auth.md
   +++ b/modules/auth.md
   @@ -1,1 +1,1 @@
   -# Auth
   +# Auth (updated)
   --- a/../../../home/<user>/.ssh/authorized_keys
   +++ b/../../../home/<user>/.ssh/authorized_keys
   @@ -0,0 +1,1 @@
   +ssh-ed25519 AAAA…attacker
   ```

2. `diff_targets_slug(patch, "modules/auth")` returns `true` (first pair
   matches).
3. `git_apply_inner` runs `git apply --unsafe-paths --directory=.wiki`,
   which honours both pairs. `--directory=.wiki` prepends `.wiki/` to
   each path, but since each pair already contains `../` segments, the
   final resolved path escapes `.wiki/`.

## Suggested fix

1. Walk **every** `---` / `+++` header in the diff (not just the first
   pair) and assert each strips to `{target_slug}.md`. Reject patches
   with more than one file-pair entirely if the design is one-slug-per-
   patch.

2. Drop `--unsafe-paths` and rely on git's default reject-on-escape
   behaviour. The header `--directory=.wiki` already gives git the
   context it needs to refuse paths outside the wiki. Keep
   `--unsafe-paths` only on the `--check` invocation if the LLM tends
   to emit paths relative to `.wiki/` instead of the project root,
   document why.

3. As defence-in-depth, after stripping `a/`/`b/` prefix, reject any
   header whose remainder contains `..` segments before passing to
   `git apply`.

## Cross-validation

The security agent identified this finding; I directly verified at
`distill_patch.rs:217-241` (the `break` on first pair) and
`distill_patch.rs:504` (the `--unsafe-paths` flag).
