import { defineConfig, devices } from "@playwright/test";

// NOTE(coral-ui frontend): the SPA is served by `coral ui serve` on
// port 38400 by convention. Tests assume the server is already running
// — see e2e/README.md for the recommended bootstrap. We do NOT spawn
// the Rust binary here because it requires a populated coral.toml +
// indexed pages, and the test author is in a better position to set
// that up than CI defaults.

const BASE_URL = process.env.CORAL_E2E_BASE_URL ?? "http://localhost:38400";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? "github" : "list",
  use: {
    baseURL: BASE_URL,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
});
