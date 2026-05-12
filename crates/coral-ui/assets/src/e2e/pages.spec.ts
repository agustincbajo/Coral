import { expect, test } from "@playwright/test";

test("pages view renders filters sidebar + table headers", async ({ page }) => {
  await page.goto("/pages");

  // The filters sidebar lives left of the main column. We assert it
  // exists by looking for the search input that's always rendered.
  await expect(
    page.locator('input[placeholder]').first(),
  ).toBeVisible({ timeout: 10_000 });

  // Table headers — copy comes from the i18n bundle. We match on a
  // small set of headers in either locale to keep the suite portable.
  const headerCandidates = [
    "Slug", // both en + es
    /Type|Tipo/,
    /Status|Estado/,
    /Confidence|Confianza/,
  ];
  for (const candidate of headerCandidates) {
    await expect(page.locator("thead").getByText(candidate).first()).toBeVisible();
  }
});
