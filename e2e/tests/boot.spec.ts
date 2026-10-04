import { expect, test } from "./fixtures";

test("asks who is signed in while the wasm is still downloading", async ({ page, app }) => {
  let statusCalls = 0;
  let statusAsked!: () => void;
  const asked = new Promise<void>((resolve) => (statusAsked = resolve));
  page.on("request", (request) => {
    if (request.url().endsWith("/doris.auth.v1.AuthService/GetStatus")) {
      statusCalls++;
      statusAsked();
    }
  });
  let askedBeforeWasm = false;
  await page.route("**/*_bg.wasm", async (route) => {
    await Promise.race([asked, new Promise((resolve) => setTimeout(resolve, 2000))]);
    askedBeforeWasm = statusCalls > 0;
    await route.continue();
  });

  await page.goto(app);

  await expect(page.getByRole("button", { name: "Skapa konto med passkey" })).toBeVisible({ timeout: 15_000 });
  expect(askedBeforeWasm).toBe(true);
  expect(statusCalls).toBe(1);
});

test("shows a progress bar until the app has loaded", async ({ page, app }) => {
  let release!: () => void;
  const released = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/*_bg.wasm", async (route) => {
    await released;
    await route.continue();
  });

  await page.goto(app, { waitUntil: "commit" });

  await expect(page.getByRole("progressbar", { name: "Laddar Doris" })).toBeVisible();
  release();
  await expect(page.getByRole("button", { name: "Skapa konto med passkey" })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByRole("progressbar")).toHaveCount(0);
});

test("the progress bar fetches no font, leaving the bandwidth to the wasm", async ({ page, app }) => {
  const fonts: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/fonts/")) fonts.push(request.url());
  });
  let release!: () => void;
  const released = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/*_bg.wasm", async (route) => {
    await released;
    await route.continue();
  });

  await page.goto(app, { waitUntil: "commit" });
  await expect(page.getByRole("progressbar", { name: "Laddar Doris" })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);

  expect(fonts).toEqual([]);
  release();
});

test("says so when the app cannot be loaded", async ({ page, app }) => {
  await page.route("**/*_bg.wasm", (route) => route.abort());

  await page.goto(app, { waitUntil: "commit" });

  await expect(page.getByText("Kunde inte ladda Doris. Ladda om sidan.")).toBeVisible();
  await expect(page.getByRole("progressbar")).toHaveCount(0);
});

test("the progress bar counts toward the wasm's real size", async ({ page, app }) => {
  const html = await (await page.request.get(app)).text();
  const [, wasm, size] = html.match(/__trunkInitializer\(init, '([^']+)', (\d+),/)!;

  const body = await (await page.request.get(app + wasm)).body();

  expect(Number(size)).toBe(body.length);
});
