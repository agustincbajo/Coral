import { expect, test } from "@playwright/test";

test("graph view renders Sigma canvas or no-webgl2 fallback", async ({
  page,
}) => {
  await page.goto("/graph");

  // Either the Sigma renderer mounts a <canvas>, or the WebGL2-
  // detection fallback renders a static notice. We accept either —
  // headless Chromium may or may not expose WebGL2 depending on the
  // host.
  const canvas = page.locator("canvas").first();
  const fallback = page.getByText(/WebGL 2|WebGL2/i).first();

  await expect.poll(
    async () => {
      const hasCanvas = (await canvas.count()) > 0;
      const hasFallback = (await fallback.count()) > 0;
      return hasCanvas || hasFallback;
    },
    { timeout: 15_000 },
  ).toBe(true);
});
