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
