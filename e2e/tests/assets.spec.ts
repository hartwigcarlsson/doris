import { expect, test } from "./fixtures";

test("the app's wasm is served as wasm and cached forever", async ({ page, app }) => {
  const wasm = page.waitForResponse((r) => r.url().endsWith("_bg.wasm"));
  await page.goto(app);

  const response = await wasm;
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toBe("application/wasm");
  expect(response.headers()["cache-control"]).toBe("public, max-age=31536000, immutable");
});

test("the page is revalidated and never framed", async ({ page, app }) => {
  const response = await page.goto(app);

  expect(response!.headers()["cache-control"]).toBe("no-cache");
  expect(response!.headers()["content-security-policy"]).toBe("frame-ancestors 'none'");
});
