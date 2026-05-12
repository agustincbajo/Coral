import { expect, test } from "@playwright/test";

test("query view shows LLM cost warning", async ({ page }) => {
  await page.goto("/query");

  // The amber banner mentions LLM tokens / proveedor configurado —
  // match either locale.
  await expect(
    page.getByText(/LLM tokens|tokens LLM/i).first(),
  ).toBeVisible({ timeout: 10_000 });
});
