import type { Page } from "@playwright/test";
import { addCompany, addSupplier, expect, register, test } from "./fixtures";

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
  for (const name of ["Resultat hittills i år", "Intäkter", "Kostnader", "Kassa och bank"]) {
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
  await expect(figure(page, "Intäkter")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Resultat hittills i år")).toContainText(/1\s250\skr/);
  await expect(figure(page, "Kassa och bank")).toContainText(/1\s250\skr/);
  const latest = figure(page, "Senaste verifikationer");
  await expect(latest.getByRole("row", { name: /^1 .* Försäljning 1\s250,00$/ })).toBeVisible();
  await expect(figure(page, "Räkenskapsåret")).toContainText(/Verifikationer\s*1/);
  await expect(figure(page, "Kostnader").locator("p").first()).toHaveText(/^0\skr$/);
});

test("the overview follows the chosen year", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${year - 1}-01-01`);
  await book(page, app, `${year - 1}-03-01`, "700");
  await book(page, app, `${year}-01-15`, "1250");
  await page.goto(app);
  // The year that contains today is chosen, not the newest or the oldest.
  await expect(page.getByLabel("Räkenskapsår")).toHaveValue(`${year}-01-01`);
  await expect(figure(page, "Intäkter")).toContainText(/1\s250\skr/);
  await page.getByLabel("Räkenskapsår").selectOption(`${year - 1}-01-01`);
  await expect(figure(page, "Intäkter")).toContainText(/700\skr/);
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
  await expect(figure(page, "Intäkter").getByRole("alert")).toBeVisible();
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

test("nothing to do says so", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
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
