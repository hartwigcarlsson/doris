import type { Page } from "@playwright/test";
import { addCompany, expect, register, test } from "./fixtures";

// Last calendar year has always ended, so it can be closed.
const last = new Date().getFullYear() - 1;
const lastStart = `${last}-01-01`;

function pdf(name: string) {
  return { name, mimeType: "application/pdf", buffer: Buffer.from(`%PDF-1.4\n% ${name}\n%%EOF\n`) };
}

async function fillVoucher(page: Page, app: string, date?: string) {
  await page.goto(`${app}/vouchers/new`);
  if (date) await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill("Kontorsmaterial");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("125");
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill("125");
}

test("a voucher is booked with its underlag, which opens in a new tab", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await fillVoucher(page, app);
  await page.getByLabel("Underlag").setInputFiles([pdf("kvitto.pdf"), pdf("faktura.pdf")]);
  await expect(page.getByText("kvitto.pdf (1 kB)")).toBeVisible();
  await page.getByRole("button", { name: "Ta bort faktura.pdf" }).click();
  await expect(page.getByText("faktura.pdf")).toHaveCount(0);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await expect(page.getByText("kvitto.pdf")).toHaveCount(0);

  await page.goto(`${app}/vouchers`);
  const first = page.getByRole("row", { name: /^1 / });
  await expect(first).toContainText("1 underlag");
  await first.getByRole("button", { name: "1", exact: true }).click();
  // Headless Chromium has no PDF viewer: the new tab downloads the blob instead of showing it.
  const download = new Promise<string>((resolve) =>
    page.on("popup", (tab) => tab.on("download", (d) => resolve(d.url()))),
  );
  await page.getByRole("button", { name: "kvitto.pdf (1 kB)" }).click();
  expect(await download).toMatch(/^blob:/);
});

test("underlag are added later, even in a closed year, and only as PDF, JPEG or PNG", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", lastStart);
  await fillVoucher(page, app, `${last}-06-01`);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await page.getByRole("banner").getByRole("link", { name: "Räkenskapsår" }).click();
  const lastYear = page.getByRole("row", { name: new RegExp(`^${lastStart}`) });
  await lastYear.getByRole("button", { name: "Stäng år" }).click();
  await lastYear.getByRole("button", { name: "Bekräfta stängning" }).click();
  await expect(lastYear).toContainText("Stängt");

  await page.goto(`${app}/vouchers`);
  await page.getByLabel("Räkenskapsår").selectOption(lastStart);
  const first = page.getByRole("row", { name: /^1 / });
  await first.getByRole("button", { name: "1", exact: true }).click();
  await page.getByLabel("Lägg till underlag till ver 1").setInputFiles([pdf("faktura.pdf")]);
  await expect(page.getByRole("button", { name: "faktura.pdf (1 kB)" })).toBeVisible();
  await expect(first).toContainText("1 underlag");

  await page
    .getByLabel("Lägg till underlag till ver 1")
    .setInputFiles([{ name: "bild.gif", mimeType: "image/gif", buffer: Buffer.from("GIF89a") }]);
  await expect(page.getByRole("alert")).toHaveText("Underlaget måste vara en PDF, JPEG eller PNG.");
});

test("picked underlag stay with the company they were picked for", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");

  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Underlag").setInputFiles([pdf("kvitto.pdf")]);
  await expect(page.getByText("kvitto.pdf (1 kB)")).toBeVisible();
  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(page.getByText("kvitto.pdf")).toHaveCount(0);
});

test("a file over 10 MB is refused when picked, before it is read", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await page.goto(`${app}/vouchers/new`);
  const big = { name: "stor.pdf", mimeType: "application/pdf", buffer: Buffer.alloc((10 << 20) + 1) };
  await page.getByLabel("Underlag").setInputFiles([big]);
  await expect(page.getByRole("alert")).toHaveText(
    "Underlaget är för stort (högst 10 MB per fil och 20 MB totalt).",
  );
  await expect(page.getByText("stor.pdf")).toHaveCount(0);
});
