import type { Page } from "@playwright/test";
import { addCompany, addSupplier, expect, goTo, register, test } from "./fixtures";

const year = new Date().getFullYear();
const figure = (page: Page, name: string) =>
  page.getByRole("main").locator("section").filter({ has: page.getByRole("heading", { name, exact: true }) });

async function book(page: Page, app: string, date: string, kronor: string) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill("Försäljning");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
}

test("a new company's overview is empty but complete", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(app);
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Exempel AB" })).toBeVisible();
  await expect(main.getByText("556016-0680 · Aktiebolag · Faktureringsmetoden")).toBeVisible();
  for (const name of ["Resultat hittills i år", "Rörelseintäkter", "Rörelsekostnader", "Kassa och bank"]) {
    await expect(figure(page, name).locator("p").first()).toHaveText(/^0\skr$/);
  }
  const fiscalYear = figure(page, "Räkenskapsåret");
  await expect(fiscalYear.getByText("Öppet")).toBeVisible();
  await expect(fiscalYear.getByRole("progressbar")).toHaveAttribute("aria-valuemax", "100");
  await expect(fiscalYear).toContainText(`${year}-01-01 – ${year}-12-31`);
  await expect(fiscalYear).toContainText(/Dag \d+ av 36[56]/);
});

test("a booked sale shows in the key figures", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1250");
  await page.goto(app);
  await expect(figure(page, "Rörelseintäkter")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Resultat hittills i år")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Kassa och bank")).toContainText(/1\s250\skr/);
  const latest = figure(page, "Senaste verifikationer");
  await expect(latest.getByRole("row", { name: /^1 .* Försäljning 1\s250,00$/ })).toBeVisible();
  await expect(figure(page, "Räkenskapsåret")).toContainText(/Verifikationer\s*1/);
  await expect(figure(page, "Rörelsekostnader").locator("p").first()).toHaveText(/^0\skr$/);
});

test("the overview follows the chosen year", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${year - 1}-01-01`);
  await book(page, app, `${year - 1}-03-01`, "700");
  await book(page, app, `${year}-01-15`, "1250");
  await page.goto(app);
  // The year that contains today is chosen, not the newest or the oldest.
  await expect(page.getByLabel("Räkenskapsår")).toHaveValue(`${year}-01-01`);
  await expect(figure(page, "Rörelseintäkter")).toContainText(/1\s250\skr/);
  await page.getByLabel("Räkenskapsår").selectOption(`${year - 1}-01-01`);
  await expect(figure(page, "Rörelseintäkter")).toContainText(/700\skr/);
  await expect(figure(page, "Räkenskapsåret")).toContainText(/Dag 365 av 365|Dag 366 av 366/);
  // The choice survives a reload.
  await page.reload();
  await expect(page.getByLabel("Räkenskapsår")).toHaveValue(`${year - 1}-01-01`);
});

test("a failed call only takes its own cards down", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.route("**/GetTrialBalance", (route) => route.abort());
  await page.goto(app);
  await expect(figure(page, "Rörelseintäkter").getByRole("alert")).toBeVisible();
  await expect(figure(page, "Räkenskapsåret")).toContainText(/Dag \d+ av/);
});

test("without a company the start page says so", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Översikt" })).toBeVisible();
  await expect(main.getByText("Du har inga företag än.")).toBeVisible();
  await expect(main.getByRole("link", { name: "Lägg till företag" })).toBeVisible();
});

const iso = (daysFromToday: number) => {
  const d = new Date();
  d.setDate(d.getDate() + daysFromToday);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

// A new company declares VAT quarterly, and the quarters of this year that
// have ended are then on the to-do list; these tests want an empty one.
async function notVatRegistered(page: Page, app: string) {
  await page.goto(`${app}/vat`);
  // Let the page load first, or its answer overwrites the choice.
  await expect(page.getByRole("link", { name: /januari–mars/ })).toBeVisible();
  await page.getByLabel("Redovisningsperiod").selectOption("not_registered");
  await expect(page.getByRole("row")).toHaveCount(1); // only the header: no periods
}

test("nothing to do says so", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await notVatRegistered(page, app);
  await page.goto(app);
  await expect(figure(page, "Att göra")).toContainText("Inget att göra just nu.");
  await expect(figure(page, "Senaste verifikationer")).toContainText("Inga verifikationer än.");
  await expect(figure(page, "Obetalda leverantörsfakturor")).toContainText("Inga obetalda leverantörsfakturor.");
});

test("an overdue supplier invoice is on the to-do list and leads to the invoices", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  // Last year too, so that the invoice date is in a year whenever this runs.
  await addCompany(page, app, "5560160680", "Exempel AB", `${year - 1}-01-01`);
  await addSupplier(page, app, "Kontorshuset AB");
  await page.goto(`${app}/supplier-invoices/new`);
  await page.getByLabel("Leverantör").selectOption({ label: "1 Kontorshuset AB" });
  await page.getByLabel("Fakturanummer").fill("20413");
  await page.getByLabel("Fakturadatum").fill(iso(-40));
  await page.getByLabel("Förfallodatum").fill(iso(-10));
  await page.getByLabel("Konto, rad 1").fill("5010");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("10000");
  await page.getByLabel("Underlag").setInputFiles([{ name: "faktura.pdf", mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4\n%%EOF\n") }]);
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("heading", { name: "Leverantörsfakturor" })).toBeVisible();

  await page.goto(app);
  const todo = figure(page, "Att göra");
  await expect(todo).toContainText("1 leverantörsfaktura har förfallit");
  await expect(todo).toContainText(/12\s500,00 kr · äldst Kontorshuset AB/);
  const unpaid = figure(page, "Obetalda leverantörsfakturor");
  await expect(unpaid).toContainText("Kontorshuset AB");
  await expect(unpaid.getByText("Förfallen", { exact: true })).toBeVisible();
  await todo.getByRole("link", { name: "Visa fakturorna" }).click();
  await expect(page).toHaveURL(/\/supplier-invoices$/);
});

test("the chart draws a bar for a month with income and none for an empty one", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1000");
  await page.goto(app);
  const chart = figure(page, "Intäkter och kostnader per månad");
  await expect(chart.getByRole("img")).toHaveAttribute("aria-label", /Intäkter och kostnader per månad/);
  await expect(chart.getByText("Intäkter", { exact: true })).toBeVisible();
  await expect(chart.getByText("Kostnader", { exact: true })).toBeVisible();
  const heights = await chart.locator("[data-month]").evaluateAll((months) =>
    months.map((m) => [m.getAttribute("data-month"), ...[...m.children].map((bar) => Math.round(bar.getBoundingClientRect().height))]),
  );
  expect(heights).toHaveLength(12);
  expect(heights[0]).toEqual([`${year}-01`, 160, 0]);
  expect(heights[1]).toEqual([`${year}-02`, 0, 0]);
  // The two series are told apart by lightness, in both schemes.
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    const [income, costs] = await chart.locator("[data-legend]").evaluateAll((els) => els.map((el) => getComputedStyle(el).backgroundColor));
    expect(income, scheme).not.toBe(costs);
    expect(income, scheme).not.toBe("rgba(0, 0, 0, 0)");
  }
});

test("the overview fits a phone in both colour schemes", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1000");
  await page.setViewportSize({ width: 390, height: 844 });
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto(app);
    await expect(figure(page, "Senaste verifikationer").getByRole("row")).toHaveCount(2);
    const wider = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
    expect(wider, scheme).toBe(false);
  }
});

test("years that cannot be listed fail the cards that need a year", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.route("**/ListFiscalYears", (route) => route.abort());
  await page.goto(app);
  for (const name of ["Rörelseintäkter", "Räkenskapsåret", "Intäkter och kostnader per månad", "Senaste verifikationer"]) {
    await expect(figure(page, name).getByRole("alert"), name).toBeVisible();
  }
  // What does not depend on a year still shows.
  await expect(figure(page, "Att göra")).toContainText("Inget att göra just nu.");
});

test("a company that cannot be fetched says so and the rest still shows", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.route("**/GetCompany", (route) => route.abort());
  await page.goto(app);
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { level: 1, name: "Exempel AB" })).toBeVisible();
  await expect(main.getByRole("alert")).toHaveCount(1);
  await expect(figure(page, "Rörelseintäkter").locator("p").first()).toHaveText(/^0\skr$/);
});

test("one slow call does not hold the others back", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await notVatRegistered(page, app);
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/GetCompany", async (route) => {
    await held;
    await route.continue();
  });
  await page.goto(app);
  // GetCompany has not answered, and everything else is already there.
  await expect(figure(page, "Rörelseintäkter").locator("p").first()).toHaveText(/^0\skr$/);
  await expect(figure(page, "Att göra")).toContainText("Inget att göra just nu.");
  await expect(page.getByRole("main").getByText("556016-0680")).toHaveCount(0);
  release();
  await expect(page.getByRole("main").getByText("556016-0680 · Aktiebolag · Faktureringsmetoden")).toBeVisible();
});

test("leaving the overview while it loads breaks nothing", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/GetTrialBalance", async (route) => {
    await held;
    await route.continue();
  });
  await page.goto(app);
  await expect(page.getByRole("main").getByRole("heading", { level: 1, name: "Exempel AB" })).toBeVisible();
  await goTo(page, "Kontoplan");
  await expect(page.getByRole("heading", { level: 1, name: "Kontoplan" })).toBeVisible();
  // The answer for the page that is gone arrives now.
  release();
  await page.waitForTimeout(500);
  await goTo(page, "Räkenskapsår");
  await expect(page.getByRole("heading", { level: 1, name: "Räkenskapsår" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("interest income is part of the result but not of the operating figures", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, `${year}-01-15`, "1250");
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(`${year}-01-20`);
  await page.getByLabel("Text").fill("Ränta");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("100");
  await page.getByLabel("Konto, rad 2").fill("8310");
  await page.getByLabel("Kredit, rad 2").fill("100");
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
  await page.goto(app);
  await expect(figure(page, "Rörelseintäkter").locator("p").first()).toHaveText(/^1\s250\skr$/);
  await expect(figure(page, "Rörelsekostnader").locator("p").first()).toHaveText(/^0\skr$/);
  const result = figure(page, "Resultat hittills i år");
  await expect(result.locator("p").first()).toHaveText(/^1\s350\skr$/);
  await expect(result).toContainText(/varav finansiella poster m\.m\. 100\skr/);
  // Without anything in class 8 the extra line is not there.
  await expect(figure(page, "Rörelseintäkter")).not.toContainText("varav");
});
