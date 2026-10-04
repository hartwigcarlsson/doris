import { addCompany, expect, register, test } from "./fixtures";
import type { Page } from "@playwright/test";

/** A date `days` from today in the browser's sense, as YYYY-MM-DD. */
function isoDate(days = 0): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

async function addEmployee(page: Page, name: string, personnummer: string, salary: string) {
  await page.getByRole("banner").getByRole("link", { name: "Anställda" }).click();
  await page.getByLabel("Namn").fill(name);
  await page.getByLabel("Personnummer").fill(personnummer);
  await page.getByLabel("Månadslön (kr)").fill(salary);
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^${name}`) })).toBeVisible();
}

/** A finalized run paying Åsa on `payDate`, with 8 000 kr tax. */
async function finalizeRun(page: Page, payDate: string) {
  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  // The form resets itself when the employees arrive; fill it only after that.
  await expect(page.getByLabel("Brutto, Åsa Öberg")).toHaveValue(/35\s000,00/);
  await page.getByLabel("Utbetalningsdag").fill(payDate);
  await page.getByLabel("Skatt, Åsa Öberg").fill("8000");
  await page.getByRole("button", { name: "Färdigställ" }).click();
  await expect(page).toHaveURL(/\/payroll-runs\/[0-9a-f-]{36}$/);
}

test("an employee is paid: previewed, finalized, booked and backed out", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000");

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await expect(page.getByLabel("Brutto, Åsa Öberg")).toHaveValue(/35\s000,00/);
  await page.getByLabel("Utbetalningsdag").fill(isoDate());
  await page.getByLabel("Skatt, Åsa Öberg").fill("8000");
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  // 35 000 kr × 31,42 % and the net pay.
  // The form row and the preview row are both named "Åsa Öberg…"; the preview comes last.
  const preview = page.getByRole("row", { name: /^Åsa Öberg/ }).last();
  await expect(preview).toContainText(/10\s997,00/);
  await expect(preview).toContainText(/27\s000,00/);

  await page.getByRole("button", { name: "Färdigställ" }).click();
  await expect(page.getByText("Att bokföra")).toBeVisible();
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("link", { name: "Ver 1" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Öppna" })).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  const voucher = page.getByRole("row", { name: /^1 / });
  await expect(voucher).toContainText("Lön");
  await voucher.getByRole("button", { name: "1" }).click();
  for (const account of ["7210", "2710", "1930", "7510", "2731"]) {
    await expect(page.getByText(new RegExp(`^${account} `))).toBeVisible();
  }

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await expect(page.getByRole("row", { name: new RegExp(isoDate()) })).toContainText("Bokförd");
  await page.getByRole("link", { name: isoDate() }).click();
  await page.getByRole("button", { name: "Backa bokföring" }).click();
  await page.getByRole("button", { name: "Bekräfta backning" }).click();
  await expect(page.getByText("Att bokföra")).toBeVisible();
  await page.getByRole("button", { name: "Öppna" }).click();
  await expect(page.getByLabel("Skatt, Åsa Öberg")).toHaveValue(/8\s000,00/);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /^2 / })).toContainText("Rättelse av ver 1");
});

test("a run for a later pay date is finalized now and booked only from that date", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000");
  const tomorrow = isoDate(1);

  await finalizeRun(page, tomorrow);

  await expect(page.getByText("Färdigställd")).toBeVisible();
  await expect(page.getByRole("button", { name: "Bokför" })).toBeDisabled();
  await expect(page.getByText(`Kan bokföras från ${tomorrow}`)).toBeVisible();
  await page.getByRole("button", { name: "Öppna" }).click();
  await page.getByLabel("Skatt, Åsa Öberg").fill("8100");
  await page.getByRole("button", { name: "Spara" }).click();
  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await expect(page.getByRole("row", { name: new RegExp(tomorrow) })).toContainText("Öppen");
});

test("a grundbok rättelse of the payroll voucher makes the run finalized again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000");
  await finalizeRun(page, isoDate());
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("link", { name: "Ver 1" })).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await page.getByRole("row", { name: /^1 / }).getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(page.getByRole("row", { name: /^1 / })).toContainText("Rättad av ver 2");

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await expect(page.getByRole("row", { name: new RegExp(isoDate()) })).toContainText("Att bokföra");
});

test("employees are refused in Swedish and edited without their personnummer", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Anställda" }).click();

  await page.getByLabel("Namn").fill("Åsa Öberg");
  await page.getByLabel("Personnummer").fill("19800101-1232");
  await page.getByLabel("Månadslön (kr)").fill("35000");
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("alert")).toHaveText("Personnumret är ogiltigt (ÅÅÅÅMMDD-NNNN).");

  await page.getByLabel("Personnummer").fill("19800101-1231");
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  const row = page.getByRole("row", { name: /^Åsa Öberg/ });
  await expect(row).toContainText("19800101-1231");
  await row.getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("Personnummer")).toHaveCount(0);
  await page.getByLabel("Månadslön (kr)").fill("36000");
  await page.getByRole("button", { name: "Spara ändringar" }).click();
  await expect(row).toContainText(/36\s000,00/);

  await row.getByRole("button", { name: "Inaktivera" }).click();
  await row.getByRole("button", { name: "Bekräfta inaktivering" }).click();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toHaveCount(0);
  await page.getByLabel("Visa inaktiva").check();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText("Inaktiv");
});
