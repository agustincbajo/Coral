# orchestra-ingest — reference consumer of Coral

A skeleton showing how a real microservice repo wires up Coral. Copy this
directory to a new repo (or use it as a template) when you want to
bootstrap an LLM-maintained wiki for a new project.

> **Status**: skeleton only — there's no real code under `src/`. The point
> is the `.wiki/` + `.github/workflows/` setup, not the microservice
> itself.

## What's here

```
examples/orchestra-ingest/
├── README.md                          # this file
├── Cargo.toml                         # placeholder microservice manifest
├── src/main.rs                        # stub
├── .wiki/
│   ├── SCHEMA.md                      # extends Coral's base SCHEMA
│   ├── index.md                       # auto-maintained
│   ├── log.md                         # append-only ingest history
│   ├── modules/
│   │   └── ingest-pipeline.md         # seed page
│   ├── concepts/
│   │   └── outbox.md                  # seed page
│   ├── flows/
│   │   └── http-to-kafka.md           # seed page
│   └── operations/
│       └── runbook.md                 # seed page
├── .coral-pins.toml                   # pinned to a stable Coral release
└── .github/
    └── workflows/
        └── wiki-maintenance.yml       # the 3 cron jobs (ingest/lint/consolidate)
```

## How to use

### 1. Copy to a new repo

```bash
cp -r examples/orchestra-ingest/ /path/to/new-repo/
cd /path/to/new-repo/
git init && git add -A && git commit -m "initial: bootstrapped from coral example"
```

### 2. Wire your Anthropic OAuth token

In your new repo's GitHub settings:
- Settings → Secrets and variables → Actions → New repository secret
- Name: `CLAUDE_CODE_OAUTH_TOKEN`
- Value: the OAuth token from `claude setup-token` (see [Coral README](../../README.md#auth-setup))

### 3. Push

The workflow at `.github/workflows/wiki-maintenance.yml` runs:
- **ingest** on every push to `main` — opens a `wiki/auto-ingest` PR if any pages changed.
- **lint** nightly — posts findings as a PR comment if there are critical issues.
- **consolidate** weekly — opens a `wiki/consolidate` PR with merge/retire/split suggestions.

## Customizing the SCHEMA

`.wiki/SCHEMA.md` extends Coral's base SCHEMA. Edit it to add project-specific
page-type rules, naming conventions, lint thresholds, etc. The LLM reads
this contract when bootstrapping/ingesting/consolidating — your
customizations propagate immediately.

## Updating the Coral pin

When a new Coral release ships:

```bash
coral validate-pin                      # confirm the new tag exists
# Edit .coral-pins.toml: bump default = "vX.Y.Z"
git commit -am "chore: bump coral pin to vX.Y.Z"
```

The workflows automatically re-resolve `coral-cli` to the pinned version
on the next CI run.
