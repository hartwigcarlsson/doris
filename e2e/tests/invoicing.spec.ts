import { addCompany, expect, register, test } from "./fixtures";
import type { Locator } from "@playwright/test";

// The Status cell: the row's buttons also say "Aktivera"/"Inaktivera".
const status = (row: Locator) => row.getByRole("cell").nth(4);

test("customers are added, edited and deactivated", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Kunder" }).click();

  await page.getByRole("button", { name: "Ny kund" }).click();
  await expect(page.getByLabel("Betalningsvillkor (dagar)")).toHaveValue("30");
  await page.getByLabel("Namn", { exact: true }).fill("Kund AB");
  await page.getByLabel("Org.nr/personnr").fill("5560360793");
  await page.getByLabel("Ort", { exact: true }).fill("Stockholm");
  await page.getByLabel("Betalningsvillkor (dagar)").fill("trettio");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("alert")).toHaveText("Betalningsvillkoret ska vara 0–365 dagar.");

  await page.getByLabel("Betalningsvillkor (dagar)").fill("10");
  await page.getByRole("button", { name: "Spara" }).click();
  const row = page.getByRole("row", { name: /^1 Kund AB 556036-0793 Stockholm/ });
  await expect(status(row)).toHaveText("Aktiv");

  await row.getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("Betalningsvillkor (dagar)")).toHaveValue("10");
  await page.getByLabel("Namn", { exact: true }).fill("Kund i Stockholm AB");
  await page.getByRole("button", { name: "Spara" }).click();
  const renamed = page.getByRole("row", { name: /^1 Kund i Stockholm AB/ });
  await expect(renamed).toBeVisible();

  await renamed.getByRole("button", { name: "Inaktivera" }).click();
  await expect(status(renamed)).toHaveText("Inaktiv");
  await renamed.getByRole("button", { name: "Aktivera" }).click();
  await expect(status(renamed)).toHaveText("Aktiv");
});

test("suppliers are added with payment details, edited and deactivated", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Leverantörer" }).click();

  await page.getByRole("button", { name: "Ny leverantör" }).click();
  await page.getByLabel("Namn", { exact: true }).fill("Lev AB");
  await page.getByLabel("Bankgiro").fill("5050-1056");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ange ett giltigt bankgironummer (7–8 siffror).");

  await page.getByLabel("Bankgiro").fill("50501055");
  await page.getByLabel("IBAN").fill("se45 5000 0000 0583 9825 7466");
  await page.getByLabel("BIC").fill("essesess");
  await page.getByRole("button", { name: "Spara" }).click();
  const row = page.getByRole("row", { name: /^1 Lev AB/ });
  await expect(row).toContainText("5050-1055");

  await row.getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("IBAN")).toHaveValue("SE45 5000 0000 0583 9825 7466");
  await expect(page.getByLabel("BIC")).toHaveValue("ESSESESS");
  await page.getByRole("button", { name: "Avbryt" }).click();

  await row.getByRole("button", { name: "Inaktivera" }).click();
  await expect(status(row)).toHaveText("Inaktiv");
});
