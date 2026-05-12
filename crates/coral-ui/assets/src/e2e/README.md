# Coral UI — Playwright E2E

These tests exercise the SPA against a live `coral ui serve` instance.
They are **local-only** for now and are not wired into CI.

## One-time setup

```bash
# Install browser binaries the first time (Chromium only).
npx playwright install --with-deps chromium
```

## Running the suite

1. From the repo root, start the backend with a populated workspace on
   the default port:

   ```bash
   coral ui serve --no-open --host 127.0.0.1 --port 38400
   ```

   The tests assume `http://localhost:38400` — override with
   `CORAL_E2E_BASE_URL=http://host:port` if needed.

2. From `crates/coral-ui/assets/src`:

   ```bash
   # Headless run (CI-style output).
   npm run test:e2e

   # Interactive UI mode (recommended while iterating on tests).
   npm run test:e2e:ui
   ```

## Tests

| File | What it asserts |
| --- | --- |
| `nav.spec.ts` | All 9 top-level nav links render and route correctly. |
| `pages.spec.ts` | Filters sidebar + table headers exist on `/pages`. |
| `graph.spec.ts` | Either a Sigma `<canvas>` or the no-WebGL2 fallback renders on `/graph`. |
| `query.spec.ts` | LLM-cost amber banner is visible on `/query`. |
| `manifest.spec.ts` | Three tabs (manifest / lock / stats) render on `/manifest`. |

## Notes

- Tests are locale-agnostic where practical (regex-match en/es copy).
- They do **not** require an LLM token; views that gate on auth still
  paint their static chrome (banners, headers, etc.).
- `playwright.config.ts` reads `CORAL_E2E_BASE_URL` so you can point at
  a remote / dev environment.
