import { addCompany, addSupplier, expect, goTo, openMenu, register, test } from "./fixtures";

// Spacing measured from the shadcn preset b1Gdz9bFY reference (mira): cards
// ~336px wide, 16px between fields, 8px from label to input, 16px from the
// last field to the submit button.
test("forms follow the preset's spacing", async ({ page, app }) => {
  await page.goto(`${app}/register`);
  await page.getByLabel("Passkeyns namn").waitFor();

  const m = await page.evaluate(() => {
    const box = (el: Element) => el.getBoundingClientRect();
    const labels = [...document.querySelectorAll("form label")];
    const inputs = [...document.querySelectorAll("form input")];
    const button = document.querySelector("form button")!;
    return {
      cardWidth: box(document.querySelector("section")!).width,
      labelToInput: box(inputs[0]).top - box(labels[0]).bottom,
      betweenFields: box(labels[1]).top - box(inputs[0]).bottom,
      lastFieldToButton: box(button).top - box(inputs[inputs.length - 1]).bottom,
    };
  });

  expect(m.labelToInput).toBe(8);
  expect(m.betweenFields).toBe(16);
  expect(m.lastFieldToButton).toBe(16);
  expect(m.cardWidth).toBeLessThanOrEqual(352);
});

const linksIn = async (panel: import("@playwright/test").Locator) =>
  (await panel.getByRole("link").allInnerTexts()).map((t) => t.trim());

test("the header is one row with grouped menus", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna Lind" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.setViewportSize({ width: 1280, height: 800 });
  const banner = page.getByRole("banner");

  const height = (await banner.boundingBox())!.height;
  expect(height).toBeGreaterThanOrEqual(48);
  expect(height).toBeLessThanOrEqual(49);
  await expect(banner.getByRole("link", { name: "Översikt" })).toBeVisible();
  await expect(banner.getByRole("link", { name: "Kunder" })).toBeVisible();
  await expect(banner.getByRole("link", { name: "Verifikationer" })).toBeHidden();

  expect(await linksIn(await openMenu(page, "Bokföring"))).toEqual(["Verifikationer", "Saldobalans", "Rapporter", "Kontoplan", "Räkenskapsår"]);
  expect(await linksIn(await openMenu(page, "Inköp"))).toEqual(["Leverantörsfakturor", "Leverantörer"]);
  // Opening one menu closed the one before it.
  await expect(banner.getByRole("link", { name: "Verifikationer" })).toBeHidden();
  expect(await linksIn(await openMenu(page, "Lön"))).toEqual(["Lönekörningar", "Anställda"]);
  const account = await openMenu(page, "Konto");
  expect(await linksIn(account)).toEqual(["Företag", "Passkeys", "Inbjudningar"]);
  await expect(account.getByRole("button", { name: "Logga ut" })).toBeVisible();
  await expect(banner.getByText("AL", { exact: true })).toBeVisible();
});

test("a menu closes on Escape, on a click outside and when its current page is chosen", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const banner = page.getByRole("banner");
  const link = banner.getByRole("link", { name: "Verifikationer" });

  await openMenu(page, "Bokföring");
  await page.keyboard.press("Escape");
  await expect(link).toBeHidden();

  await openMenu(page, "Bokföring");
  await page.getByRole("main").click({ position: { x: 5, y: 5 } });
  await expect(link).toBeHidden();

  await goTo(page, "Verifikationer");
  await expect(page).toHaveURL(/\/vouchers$/);
  await expect(link).toBeHidden();
  // Already on the page: the path does not change, and the menu still closes.
  await goTo(page, "Verifikationer");
  await expect(link).toBeHidden();
});

test("the header marks where the current page lives", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers/new`);
  const banner = page.getByRole("banner");
  await expect(banner.locator("summary", { hasText: "Bokföring" })).toHaveAttribute("data-current", "true");
  await expect(banner.locator("summary", { hasText: "Inköp" })).not.toHaveAttribute("data-current", "true");
  await page.goto(`${app}/vouchers`);
  const panel = await openMenu(page, "Bokföring");
  await expect(panel.getByRole("link", { name: "Verifikationer" })).toHaveAttribute("aria-current", "page");
});

test("without a company there is no main menu", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const banner = page.getByRole("banner");
  await expect(banner.getByRole("link", { name: "Lägg till företag" })).toBeVisible();
  await expect(banner.locator("summary", { hasText: "Bokföring" })).toHaveCount(0);
  await expect(banner.locator("summary", { hasText: "Konto" })).toHaveCount(1);
});

test("nothing spills out of the header, with a menu open or not", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    const picker = await page.getByLabel("Aktivt företag").boundingBox();
    expect(picker!.width, `picker at ${width}px`).toBeGreaterThanOrEqual(150);
    for (const menu of [null, "Bokföring", "Konto"] as const) {
      if (menu) await openMenu(page, menu);
      const wider = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
      expect(wider, `page scrolls sideways at ${width}px with ${menu ?? "no menu"} open`).toBe(false);
      await page.keyboard.press("Escape");
    }
  }
});

test("the menu panel follows the colour scheme", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const background = async () => (await openMenu(page, "Bokföring")).evaluate((el) => getComputedStyle(el).backgroundColor);
  await page.emulateMedia({ colorScheme: "light" });
  const light = await background();
  await page.keyboard.press("Escape");
  await page.emulateMedia({ colorScheme: "dark" });
  expect(await background()).not.toBe(light);
});

test("Verifikationer follows the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await goTo(page, "Verifikationer");
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Verifikationer" })).toBeVisible();
  // The action is a button-shaped link with the primary colour, not a text link.
  const add = main.getByRole("link", { name: "Ny verifikation" });
  expect((await add.boundingBox())!.height).toBe(28);
  expect(await add.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
  // The table sits in a card.
  await expect(main.locator("section").filter({ has: page.getByRole("table") })).toHaveCount(1);
  await add.click();
  await expect(main.getByRole("heading", { level: 1, name: "Ny verifikation" })).toBeVisible();
});

/** A view has one h1 in main, and every table in main sits in a card. */
async function expectDesign(page: import("@playwright/test").Page, heading: string | RegExp) {
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: heading })).toBeVisible();
  await expect(main.getByRole("heading", { level: 1 })).toHaveCount(1);
  const loose = await main.evaluate((el) => [...el.querySelectorAll("table")].filter((t) => !t.closest("section")).length);
  expect(loose, "tables outside a card").toBe(0);
}

test("the bookkeeping views follow the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  for (const [link, heading] of [
    ["Saldobalans", "Saldobalans"],
    ["Rapporter", "Resultat- och balansräkning"],
    ["Kontoplan", "Kontoplan"],
    ["Räkenskapsår", "Räkenskapsår"],
  ] as const) {
    await goTo(page, link);
    await expectDesign(page, heading);
  }
  // Öppet is a badge, not bare cell text.
  const status = page.getByRole("main").getByText("Öppet", { exact: true });
  expect(await status.evaluate((el) => getComputedStyle(el).borderRadius)).not.toBe("0px");
  await page.getByRole("link", { name: "Ingående balanser" }).click();
  await expectDesign(page, /Ingående balanser/);
  await page.goto(`${app}/trial-balance/1930`);
  await expectDesign(page, /^1930/);
});

test("the purchase and customer views follow the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Kontorshuset AB");
  await expectDesign(page, "Leverantörer");
  const active = page.getByRole("row", { name: /Kontorshuset AB/ }).getByText("Aktiv", { exact: true });
  expect(await active.evaluate((el) => getComputedStyle(el).borderRadius)).not.toBe("0px");
  await goTo(page, "Kunder");
  await expectDesign(page, "Kunder");
  await goTo(page, "Leverantörsfakturor");
  await expectDesign(page, "Leverantörsfakturor");
  const add = page.getByRole("main").getByRole("link", { name: "Ny leverantörsfaktura" });
  expect((await add.boundingBox())!.height).toBe(28);
  await add.click();
  await expectDesign(page, "Ny leverantörsfaktura");
});

test("the payroll views follow the design", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await goTo(page, "Anställda");
  await expectDesign(page, "Anställda");
  await goTo(page, "Lönekörningar");
  await expectDesign(page, "Lönekörningar");
  const add = page.getByRole("main").getByRole("link", { name: "Ny lönekörning" });
  expect((await add.boundingBox())!.height).toBe(28);
  await add.click();
  await expectDesign(page, /lönekörning/i);
});

test("the account views follow the design, with narrow left-aligned forms", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.setViewportSize({ width: 1280, height: 800 });
  await expectDesign(page, "Översikt");
  await goTo(page, "Företag");
  await expectDesign(page, "Företag");
  await page.getByRole("main").getByRole("link", { name: "Lägg till företag" }).click();
  await expectDesign(page, "Lägg till företag");
  const { card, heading } = await page.getByRole("main").evaluate((main) => ({
    card: main.querySelector("section")!.getBoundingClientRect(),
    heading: main.querySelector("h1")!.getBoundingClientRect(),
  }));
  expect(card.width).toBeLessThanOrEqual(352);
  expect(card.left).toBe(heading.left);
  await goTo(page, "Passkeys");
  await expectDesign(page, "Passkeys");
  await goTo(page, "Inbjudningar");
  await expectDesign(page, "Inbjudningar");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await expectDesign(page, "Exempel AB");
});

test("every signed-in view has one h1 and no page scrolls sideways", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const paths = [
    "/", "/companies", "/companies/new", "/accounts", "/vouchers", "/vouchers/new", "/customers", "/suppliers",
    "/supplier-invoices", "/supplier-invoices/new", "/trial-balance", "/trial-balance/1930", "/financial-statements",
    "/fiscal-years", "/opening-balances", "/employees", "/payroll-runs", "/payroll-runs/new", "/settings/passkeys",
    "/admin/invitations",
  ];
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    for (const path of paths) {
      await page.goto(`${app}${path}`);
      await expect.soft(page.getByRole("main").getByRole("heading", { level: 1 }), `${path} at ${width}px`).toHaveCount(1);
      const wider = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
      expect.soft(wider, `${path} scrolls sideways at ${width}px`).toBe(false);
    }
  }
});
