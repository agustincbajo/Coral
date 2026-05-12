import { expect, test } from "@playwright/test";

// Routes that should be reachable from the top-level nav. The i18n
// labels come from en.json / es.json — we match on the route URL
// post-click rather than the rendered label so the test is locale-
// agnostic.
const ROUTES = [
  "/pages",
  "/graph",
  "/query",
  "/manifest",
  "/interfaces",
  "/drift",
  "/affected",
  "/tools",
  "/guarantee",
];

test("nav exposes all primary routes", async ({ page }) => {
  await page.goto("/");
  // Wait for the SPA to settle on its default route (`/pages`).
  await expect(page).toHaveURL(/\/pages$/);

  const links = page.locator("header nav a");
  await expect(links).toHaveCount(ROUTES.length);
});

for (const route of ROUTES) {
  test(`navigate to ${route}`, async ({ page }) => {
    await page.goto("/");
    await page.locator(`header nav a[href="${route}"]`).first().click();
    await expect(page).toHaveURL(new RegExp(`${route}$`));
  });
}
