# Verifying Coral release artifacts (SLSA build provenance)

Every Coral release since v0.30.0 ships with a **SLSA L3-equivalent**
build provenance attestation. The attestation is an in-toto Sigstore
record that proves which CI workflow built which artifact, signed by
the Sigstore public-good instance via GitHub's OIDC identity.

Useful when you want to confirm that the binary you're about to run
was produced by `.github/workflows/release.yml` on a specific
commit — not tampered with after publication, not a typo-squat from
another repo.

---

## TL;DR — verify a Coral binary in 2 commands

```bash
# 1. Download the artifact you want to install.
curl -fsSL -O https://github.com/agustincbajo/Coral/releases/download/v0.34.1/coral-v0.34.1-x86_64-unknown-linux-gnu.tar.gz

# 2. Verify its provenance against the agustincbajo/Coral repository.
gh attestation verify coral-v0.34.1-x86_64-unknown-linux-gnu.tar.gz \
  --repo agustincbajo/Coral
```

Expected output:

```
Loaded digest sha256:... for file://coral-v0.34.1-x86_64-unknown-linux-gnu.tar.gz
Loaded 1 attestation from GitHub API
The following policy criteria will be enforced:
  - Predicate type must match:................. https://slsa.dev/provenance/v1
  - Source Repository Owner URI must match:.... https://github.com/agustincbajo
  - Source Repository URI must match:.......... https://github.com/agustincbajo/Coral
  - Subject Alternative Name must match regex:. ^https://github.com/agustincbajo/Coral/.github/workflows/release.yml@.+

✓ Verification succeeded!
```

Any other output means **do not run the binary**. Re-download from the
official release page and try again.

---

## What the attestation proves

The attestation is an **in-toto statement** with a
`https://slsa.dev/provenance/v1` predicate. It binds:

- The exact **SHA-256 digest** of the artifact
- The **workflow file path** that produced it (`release.yml`)
- The **commit SHA** the workflow ran on
- The **GitHub Actions runner identity** (a short-lived OIDC token, no
  long-lived signing key to leak)

It does **NOT** prove:

- That the source code at the commit SHA is benign (read the code)
- That the binary's behavior matches its README (run the smoke tests)
- That subsequent transit (CDN, proxy, OS package mirror) was clean
  (the digest binding catches modifications, but only if you verify
  before extraction)

---

## Verifying on macOS / Windows

The `gh` CLI works identically on all three platforms; only the file
suffix changes:

| Platform | Asset extension |
|---|---|
| Linux x86_64 | `coral-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| macOS x86_64 | `coral-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| macOS aarch64 | `coral-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `coral-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

Plus a sibling `.sha256` file with the expected SHA-256 in
`<hex>  <filename>` shape (matching `shasum -a 256` output).

**Windows note:** the `gh attestation verify` command requires
`gh` >= 2.45.0. The same `iwr install.ps1 | iex` flow that installs
Coral can verify the binary post-install — `coral self-upgrade
--check-only` will run the attestation check internally when v0.35
ships (tracked).

---

## Verifying without `gh` — `cosign` route

If you're in a CI environment without the `gh` CLI but with
`cosign`, the same Sigstore bundle works:

```bash
# 1. Fetch the attestation bundle from the GitHub API.
curl -fsSL \
  "https://api.github.com/repos/agustincbajo/Coral/attestations/sha256:$(sha256sum coral-v0.34.1-x86_64-unknown-linux-gnu.tar.gz | cut -d' ' -f1)" \
  > attestation.json

# 2. Extract the bundle and verify it against the artifact.
jq -r '.attestations[0].bundle' attestation.json > attestation.bundle
cosign verify-blob-attestation \
  --bundle attestation.bundle \
  --new-bundle-format \
  --certificate-identity-regexp '^https://github.com/agustincbajo/Coral/' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  coral-v0.34.1-x86_64-unknown-linux-gnu.tar.gz
```

`cosign` is the right verifier if your supply-chain policy already
trusts the Sigstore public-good instance directly rather than going
through GitHub's API.

---

## What if verification fails?

| Symptom | Cause | Fix |
|---|---|---|
| `no attestations found` | Pre-v0.30.0 release, or the API hasn't propagated yet | Wait 60s and retry; or use the GitHub Release page's manual download |
| `Source Repository URI must match` failure | You're verifying a fork or a re-uploaded binary | Re-download from `agustincbajo/Coral`'s official releases |
| `subject did not match` failure | Artifact was modified after publication | Treat as untrusted; re-download from the official release page |
| `gh: command not found` | gh CLI not installed | Install from `https://cli.github.com` or use the `cosign` route above |

---

## How the attestation is produced (CI side)

`release.yml` contains a `provenance` job at the end of every release:

```yaml
provenance:
  name: Build provenance attestation
  needs: release
  permissions:
    id-token: write
    contents: read
    attestations: write
  steps:
    - uses: actions/download-artifact@v4
      with:
        path: dist
        merge-multiple: true
    - uses: actions/attest-build-provenance@v2
      with:
        subject-path: |
          dist/coral-v*-x86_64-unknown-linux-gnu.tar.gz
          dist/coral-v*-x86_64-apple-darwin.tar.gz
          dist/coral-v*-aarch64-apple-darwin.tar.gz
          dist/coral-v*-x86_64-pc-windows-msvc.zip
          dist/coral-v*.mcpb
```

The `id-token: write` permission lets the workflow request a short-lived
OIDC token from GitHub, which it then exchanges with Sigstore for a
signing certificate. The resulting signed bundle is uploaded to the
GitHub Attestations API and indexed by digest. No long-lived secrets
participate.

---

## References

- [SLSA spec v1.0](https://slsa.dev/spec/v1.0/)
- [in-toto attestation framework](https://github.com/in-toto/attestation/blob/main/spec/README.md)
- [`gh attestation verify` docs](https://cli.github.com/manual/gh_attestation_verify)
- [`actions/attest-build-provenance`](https://github.com/actions/attest-build-provenance)
- [Sigstore public-good instance](https://docs.sigstore.dev/about/threat-model/)
