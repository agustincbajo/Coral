import { expect, test } from "@playwright/test";

test("manifest view exposes three tabs", async ({ page }) => {
  await page.goto("/manifest");

  // Radix Tabs render <button role="tab">. We expect three (manifest,
  // lock, stats).
  const tabs = page.getByRole("tab");
  await expect(tabs).toHaveCount(3, { timeout: 10_000 });
});
