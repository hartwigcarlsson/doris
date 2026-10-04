import type { Locator, Page } from "@playwright/test";
import { addCompany, addCustomer, expect, register, test } from "./fixtures";

// The Status cell; the row's buttons have words of their own.
const status = (row: Locator) => row.getByRole("cell").nth(5);

function pdf(name: string) {
  return { name, mimeType: "application/pdf", buffer: Buffer.from(`%PDF-1.4\n% ${name}\n%%EOF\n`) };
}

async function registerInvoice(page: Page, app: string, invoiceNumber?: string) {
  await page.goto(`${app}/customer-invoices/new`);
  await page.getByLabel("Kund", { exact: true }).selectOption({ label: "1 Kund AB" });
  if (invoiceNumber) await page.getByLabel("Fakturanummer").fill(invoiceNumber);
  await page.getByLabel("Konto, rad 1").fill("3001");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await expect(page.getByText(/Att betala 1\s000,00/)).toBeVisible();
  await page.getByLabel("Underlag").setInputFiles([pdf("faktura.pdf")]);
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("heading", { name: "Kundfakturor" })).toBeVisible();
}

test("a customer invoice gets the proposed number, is booked, paid, reversed and paid again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCustomer(page, app, "Kund AB", "10");
  await page.getByRole("banner").getByRole("link", { name: "Kundfakturor" }).click();
  await page.getByRole("link", { name: "Ny kundfaktura" }).click();
  await expect(page.getByLabel("Fakturanummer")).toHaveValue("1");
  await page.getByLabel("Kund", { exact: true }).selectOption({ label: "1 Kund AB" });
  await page.getByLabel("Fakturadatum").fill(`${new Date().getFullYear()}-01-05`);
  await expect(page.getByLabel("Förfallodatum")).toHaveValue(`${new Date().getFullYear()}-01-15`);
  await registerInvoice(page, app, "1017");

  const row = page.getByRole("row", { name: /^1017 Kund AB/ });
  await expect(status(row)).toHaveText("Obetald");
  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Kundfaktura 1017, Kund AB/ })).toContainText("1 underlag");

  await page.getByRole("banner").getByRole("link", { name: "Kundfakturor" }).click();
  await page.getByLabel("Visa betalda och makulerade").check();
  await row.getByRole("button", { name: "Registrera inbetalning" }).click();
  await page.getByRole("button", { name: "Bekräfta inbetalning" }).click();
  await expect(status(row)).toHaveText("Betald");
  await row.getByRole("button", { name: "Ångra betalning" }).click();
  await page.getByLabel("Anledning").fill("Fel konto");
  await page.getByRole("button", { name: "Bekräfta ångring" }).click();
  await expect(status(row)).toHaveText("Obetald");
  await row.getByRole("button", { name: "Registrera inbetalning" }).click();
  await page.getByRole("button", { name: "Bekräfta inbetalning" }).click();
  await expect(status(row)).toHaveText("Betald");

  await page.getByRole("link", { name: "Ny kundfaktura" }).click();
  await expect(page.getByLabel("Fakturanummer")).toHaveValue("1018");
});

test("a cancelled invoice's number cannot be used again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCustomer(page, app, "Kund AB");
  await registerInvoice(page, app);

  const row = page.getByRole("row", { name: /^1 Kund AB/ });
  await row.getByRole("button", { name: "Makulera" }).click();
  await page.getByLabel("Anledning").fill("Fel kund");
  await page.getByRole("button", { name: "Bekräfta makulering" }).click();
  await expect(row).toHaveCount(0);

  await page.goto(`${app}/customer-invoices/new`);
  await page.getByLabel("Kund", { exact: true }).selectOption({ label: "1 Kund AB" });
  await page.getByLabel("Fakturanummer").fill("1");
  await page.getByLabel("Konto, rad 1").fill("3001");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("alert")).toHaveText(/Fakturanumret är redan använt/);
});

test("switching company clears the customer invoice form", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addCustomer(page, app, "Kund AB");
  await page.goto(`${app}/customer-invoices/new`);
  await page.getByLabel("Fakturanummer").fill("77");
  await page.getByLabel("Konto, rad 1").fill("3001");
  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });
  await expect(page.getByLabel("Fakturanummer")).toHaveValue("1");
  await expect(page.getByLabel("Konto, rad 1")).toHaveValue("");
  await expect(page.getByLabel("Kund", { exact: true })).toHaveValue("");
});

test("under kontantmetoden only the payment is booked, and Räkenskapsår warns while unpaid", async ({ page, app }) => {
  const warning = /Det finns obetalda kund- eller leverantörsfakturor/;
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", undefined, "Kontantmetoden");
  await addCustomer(page, app, "Kund AB");
  await registerInvoice(page, app);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Kundfaktura/ })).toHaveCount(0);
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByText(warning)).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Kundfakturor" }).click();
  await page.getByRole("row", { name: /^1 Kund AB/ }).getByRole("button", { name: "Registrera inbetalning" }).click();
  await page.getByRole("button", { name: "Bekräfta inbetalning" }).click();
  await expect(page.getByRole("row", { name: /^1 Kund AB/ })).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Kundfaktura 1, Kund AB/ })).toContainText("1 underlag");
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByRole("heading", { name: "Räkenskapsår" })).toBeVisible();
  await expect(page.getByText(warning)).toHaveCount(0);
});
