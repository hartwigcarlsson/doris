import type { Page } from "@playwright/test";
import { addCompany, expect, goTo, register, test } from "./fixtures";

// Last calendar year has always ended, so it can be closed.
const last = new Date().getFullYear() - 1;
const lastStart = `${last}-01-01`;
const nextStart = `${last + 1}-01-01`;

async function book(page: Page, app: string, date: string, kronor: string) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill("Försäljning");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  await page.getByRole("button", { name: "Bokför" }).click();
}

test("a year opens with balances, closes with its result and reopens", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", lastStart);

  await goTo(page, "Räkenskapsår");
  await page.getByRole("link", { name: "Ingående balanser" }).click();
  await expect(page.getByRole("heading", { name: `Ingående balanser ${lastStart}` })).toBeVisible();
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("10000");
  await page.getByLabel("Konto, rad 2").fill("2081");
  await page.getByLabel("Kredit, rad 2").fill("10000");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("status")).toHaveText("Ingående balanser sparade");
  // They come back when the page is opened again.
  await page.reload();
  // amount() groups with a no-break space; \s matches it.
  await expect(page.getByLabel("Debet, rad 1")).toHaveValue(/^10\s000,00$/);

  await book(page, app, `${last}-06-01`, "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  await goTo(page, "Räkenskapsår");
  const lastYear = page.getByRole("row", { name: new RegExp(`^${lastStart}`) });
  await lastYear.getByRole("button", { name: "Stäng år" }).click();
  await lastYear.getByRole("button", { name: "Bekräfta stängning" }).click();
  await expect(page.getByRole("status")).toHaveText("Räkenskapsåret stängt. Resultatet bokfördes som ver 2.");
  await expect(lastYear).toContainText("Stängt");

  await book(page, app, `${last}-06-02`, "100");
  await expect(page.getByRole("alert")).toHaveText("Räkenskapsåret är stängt. Bokför i ett öppet år eller öppna året igen.");

  await page.goto(`${app}/trial-balance?fy=${nextStart}`);
  await expect(page.getByRole("row", { name: /^1930 / })).toContainText("11 250,00");
  await expect(page.getByRole("row", { name: /^2099 / })).toContainText("-1 250,00");
  await expect(page.getByText(/preliminära/)).toHaveCount(0);
  // An account with only an opening balance shows it in its huvudbok.
  await page.getByRole("link", { name: "2081", exact: true }).click();
  await expect(page.getByRole("row", { name: /Ingående balans/ })).toContainText("-10 000,00");
  await expect(page.getByText("Inga transaktioner på kontot under räkenskapsåret.")).toHaveCount(0);

  await goTo(page, "Räkenskapsår");
  await lastYear.getByRole("button", { name: "Öppna igen" }).click();
  await lastYear.getByLabel("Anledning").fill("Glömd faktura");
  await lastYear.getByRole("button", { name: "Bekräfta", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Räkenskapsåret öppnat igen.");
  await expect(lastYear).toContainText("Öppet");

  await page.goto(`${app}/trial-balance?fy=${nextStart}`);
  await expect(page.getByText("Föregående räkenskapsår är inte stängt, så de ingående balanserna är preliminära.")).toBeVisible();

  await page.goto(`${app}/vouchers`);
  await page.getByLabel("Räkenskapsår").selectOption(lastStart);
  await expect(page.getByRole("row", { name: /^3 / })).toContainText("Rättelse av ver 2");
});

test("the fiscal years follow the active company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB", nextStart);
  await addCompany(page, app, "5560160680", "Exempel AB", lastStart);
  await goTo(page, "Räkenskapsår");
  const lastYear = page.getByRole("row", { name: new RegExp(`^${lastStart}`) });
  await lastYear.getByRole("button", { name: "Stäng år" }).click();
  await lastYear.getByRole("button", { name: "Bekräfta stängning" }).click();
  await expect(page.getByRole("status")).toHaveText("Räkenskapsåret stängt.");

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(page.getByRole("status")).toHaveCount(0);
  await expect(lastYear).toHaveCount(0);
  await expect(page.getByRole("row", { name: new RegExp(`^${nextStart}`) })).toContainText("Öppet");
});
