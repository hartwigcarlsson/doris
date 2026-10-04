import type { Locator, Page } from "@playwright/test";
import { addCompany, addSupplier, expect, register, test } from "./fixtures";

// The Status cell; the row's buttons have words of their own.
const status = (row: Locator) => row.getByRole("cell").nth(6);

function pdf(name: string) {
  return { name, mimeType: "application/pdf", buffer: Buffer.from(`%PDF-1.4\n% ${name}\n%%EOF\n`) };
}

async function registerInvoice(page: Page, app: string, invoiceNumber: string) {
  await page.goto(`${app}/supplier-invoices/new`);
  await page.getByLabel("Leverantör").selectOption({ label: "1 Lev AB" });
  await page.getByLabel("Fakturanummer").fill(invoiceNumber);
  await page.getByLabel("Konto, rad 1").fill("5410");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await expect(page.getByLabel("Moms", { exact: true })).toHaveValue("200,00");
  await expect(page.getByText(/Att betala 1\s000,00/)).toBeVisible();
  await page.getByLabel("Underlag").setInputFiles([pdf("faktura.pdf")]);
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("heading", { name: "Leverantörsfakturor" })).toBeVisible();
}

test("a supplier invoice is registered with its underlag, paid, reversed and paid again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Lev AB");
  await page.getByRole("banner").getByRole("link", { name: "Leverantörsfakturor" }).click();
  await registerInvoice(page, app, "F-4711");

  const row = page.getByRole("row", { name: /^1 Lev AB F-4711/ });
  await expect(status(row)).toHaveText("Obetald");
  await row.getByRole("button", { name: "Detaljer" }).click();
  await expect(page.getByText("faktura.pdf")).toBeVisible();
  await expect(page.getByText("Bankgiro 5050-1055")).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Leverantörsfaktura 1, Lev AB \(F-4711\)/ })).toContainText("1 underlag");

  await page.getByRole("banner").getByRole("link", { name: "Leverantörsfakturor" }).click();
  await page.getByLabel("Visa betalda och makulerade").check();
  await row.getByRole("button", { name: "Betala" }).click();
  await expect(page.getByLabel("Betalkonto")).toHaveValue("1930");
  await page.getByRole("button", { name: "Bekräfta betalning" }).click();
  await expect(status(row)).toHaveText("Betald");

  await row.getByRole("button", { name: "Ångra betalning" }).click();
  await page.getByLabel("Anledning").fill("Fel konto");
  await page.getByRole("button", { name: "Bekräfta ångring" }).click();
  await expect(status(row)).toHaveText("Obetald");

  await row.getByRole("button", { name: "Betala" }).click();
  await page.getByRole("button", { name: "Bekräfta betalning" }).click();
  await expect(status(row)).toHaveText("Betald");
});

test("a supplier invoice is cancelled and the same number registered again", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Lev AB");
  await registerInvoice(page, app, "F-1");

  const first = page.getByRole("row", { name: /^1 Lev AB F-1/ });
  await first.getByRole("button", { name: "Makulera" }).click();
  await page.getByLabel("Anledning").fill("Dubbelregistrerad");
  await page.getByRole("button", { name: "Bekräfta makulering" }).click();
  await expect(first).toHaveCount(0);
  await page.getByLabel("Visa betalda och makulerade").check();
  await expect(status(first)).toHaveText("Makulerad");

  await registerInvoice(page, app, "F-1");
  await expect(status(page.getByRole("row", { name: /^2 Lev AB F-1/ }))).toHaveText("Obetald");
});

test("a duplicate invoice number shows a Swedish error", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addSupplier(page, app, "Lev AB");
  await registerInvoice(page, app, "F-1");
  await page.goto(`${app}/supplier-invoices/new`);
  await page.getByLabel("Leverantör").selectOption({ label: "1 Lev AB" });
  await page.getByLabel("Fakturanummer").fill("F-1");
  await page.getByLabel("Konto, rad 1").fill("5410");
  await page.getByLabel("Belopp exkl. moms, rad 1").fill("800");
  await page.getByRole("button", { name: "Registrera" }).click();
  await expect(page.getByRole("alert")).toHaveText("Den här fakturan från leverantören är redan registrerad.");
});

test("under kontantmetoden only the payment is booked, and Räkenskapsår warns while unpaid", async ({ page, app }) => {
  const warning = /Det finns obetalda leverantörsfakturor/;
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", undefined, "Kontantmetoden");
  await addSupplier(page, app, "Lev AB");
  await registerInvoice(page, app, "F-4711");

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Leverantörsfaktura/ })).toHaveCount(0);
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByText(warning)).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Leverantörsfakturor" }).click();
  await page.getByRole("row", { name: /^1 Lev AB F-4711/ }).getByRole("button", { name: "Betala" }).click();
  await page.getByRole("button", { name: "Bekräfta betalning" }).click();
  await expect(page.getByRole("row", { name: /^1 Lev AB F-4711/ })).toHaveCount(0);

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(page.getByRole("row", { name: /Leverantörsfaktura 1, Lev AB \(F-4711\)/ })).toContainText("1 underlag");
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  await expect(page.getByRole("heading", { name: "Räkenskapsår" })).toBeVisible();
  await expect(page.getByText(warning)).toHaveCount(0);
});
