---
title: "build: stable-x86_64-pc-windows-gnu fails without `dlltool.exe` (no docs, no CI coverage)"
severity: Low
labels: docs, build, windows
confidence: 5
status: PARTIALLY RESOLVED on branch audit/fixes-v0.30.0
---

## Resolution status (2026-05-11)

- **Docs**: README "Install / Build from source" now has a Windows
  subsection with both toolchain paths (MSVC + VS Build Tools, OR MinGW
  via MSYS2) and the Git Bash `link.exe` PATH-shadow gotcha.
- **CI**: `windows-latest` added to the `cross-platform-smoke` matrix
  in `.github/workflows/ci.yml`.
- **Local repro on the audit machine**: resolved by adding
  `C:\msys64\mingw64\bin` to the User PATH (MSYS2 was already
  installed; the dlltool was just not on PATH). After that, the build
  exposed two small portability nits in pre-existing code that the
  audit branch also fixes (`#[cfg(unix)]` gating on
  `tiny_http::ListenAddr::Unix` and on the `forever_yes_script`-using
  contract tests). With those gates, all 29 new audit-fix regression
  tests pass on stable-x86_64-pc-windows-gnu.

## Open follow-up (out of scope of this branch)

~30 pre-existing tests still fail on Windows because they invoke
`/bin/echo`, `.sh` scripts, `pdftotext`, hard-coded `/usr/...` paths,
or assume `Instant::now()` > 60 s of uptime. They were never reachable
on Ubuntu+macOS CI. Track as a separate "Windows test portability"
issue rather than expanding the audit branch.

---

# Original report


## Summary

On a fresh Windows toolchain with `rustup default = stable-x86_64-pc-windows-gnu`
(the rustup installer's default host on Windows), `cargo build --workspace` fails
during the very first dependency compilation:

```
error: error calling dlltool 'dlltool.exe': program not found
error: could not compile `getrandom` (lib) due to 1 previous error
error: could not compile `windows-sys` (lib) due to 1 previous error
```

`dlltool.exe` is part of MinGW-w64 binutils, which is **not** shipped with the
rustup `stable-x86_64-pc-windows-gnu` toolchain (rustup only ships `gcc-ld`,
`libgcc_s_seh-1.dll`, `libwinpthread-1.dll`, `rust-lld.exe`, `rust-objcopy.exe`,
`self-contained/`, `wasm-component-ld.exe` under `…\rustlib\…\bin`).

## Why it matters

- `README.md:1925-1927` ("Can I use Coral on Windows?") explicitly invites users
  to compile Coral on Windows and "File a bug if you hit one." This is that bug.
- The CI matrix in `.github/workflows/ci.yml` covers `ubuntu-latest` and
  `macos-latest` only — there is no Windows smoke build, so this regression
  is invisible to the project.
- The MSVC fallback also fails in a common Windows dev setup because Git Bash's
  `C:\Program Files\Git\usr\bin\link.exe` (a coreutils hardlink tool) shadows
  MSVC's `link.exe` on `PATH`. The link error
  (`link: extra operand …rcgu.o`) is undiagnostic and points a new user
  nowhere.

## Repro

1. Install Rust on Windows via `rustup-init.exe` accepting defaults
   (host = `x86_64-pc-windows-gnu`).
2. `git clone https://github.com/agustincbajo/Coral && cd Coral`
3. `cargo build --workspace`

Expected: workspace builds.
Actual: `error: error calling dlltool 'dlltool.exe': program not found`.

## Suggested fix (cheapest first)

1. **Docs**: in the README "Install" section, document the Windows prereqs
   explicitly: either (a) `rustup default stable-x86_64-pc-windows-msvc` +
   "Build Tools for Visual Studio (C++ workload)", or (b) install
   MinGW-w64 (e.g. via `winget install MartinStorsjo.LLVM-MinGW`) so
   `dlltool.exe` is on PATH.
2. **CI**: add `windows-latest` (MSVC) to the `cross-platform-smoke` matrix
   in `.github/workflows/ci.yml`. It already builds and runs `coral init`
   on `ubuntu` / `macos`; one more matrix entry would have caught this.
3. **Optional polish**: a one-line `windows-setup-check` script under
   `scripts/` that verifies `link.exe` or `dlltool.exe` is resolvable before
   `cargo build`.

## Validation

I confirmed both failure modes on the auditing host:

- `where link.exe` → `C:\Program Files\Git\usr\bin\link.exe` (coreutils,
  not MSVC).
- `where dlltool` → not found.
- `ls ~/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/`
  contains `gcc-ld, libgcc_s_seh-1.dll, libwinpthread-1.dll, rust-lld.exe,
  rust-objcopy.exe, self-contained, wasm-component-ld.exe` — no dlltool.
