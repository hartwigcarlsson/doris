import { addCompany, expect, register, test } from "./fixtures";
import type { Page } from "@playwright/test";

function today(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

async function addEmployee(page: Page, name: string, personnummer: string, salary: string, tax: () => Promise<void>) {
  await page.getByRole("banner").getByRole("link", { name: "Anställda" }).click();
  await page.getByLabel("Namn").fill(name);
  await page.getByLabel("Personnummer").fill(personnummer);
  await page.getByLabel("Månadslön (kr)").fill(salary);
  await tax();
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^${name}`) })).toBeVisible();
}

const percent = (page: Page, p: string) => async () => {
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Fast procent" });
  await page.getByLabel("Procent").fill(p);
};
const none = (page: Page) => async () => {
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Ingen (skatten skrivs in för hand)" });
};

test("a fixed percentage computes the tax, and a typed tax is manual", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Bo Ek", "19850709-9870", "30000", percent(page, "30"));
  await expect(page.getByRole("row", { name: /^Bo Ek/ })).toContainText("30 %");

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await expect(page.getByLabel("Skatt, Bo Ek")).toHaveAttribute("placeholder", "30 %");
  await page.getByLabel("Utbetalningsdag").fill(today());
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  const preview = page.getByRole("row", { name: /^Bo Ek/ }).last();
  await expect(preview).toContainText(/9\s000,00/);
  await expect(preview).toContainText("30 %");

  await page.getByLabel("Skatt, Bo Ek").fill("8500");
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  await expect(page.getByRole("row", { name: /^Bo Ek/ }).last()).toContainText("Manuell");
});

test("an employee without a setting needs a typed tax", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000", none(page));

  await page.getByRole("banner").getByRole("link", { name: "Lönekörningar" }).click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await page.getByLabel("Utbetalningsdag").fill(today());
  await page.getByRole("button", { name: "Förhandsgranska" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ange skatt för Åsa Öberg.");
});

test("an employee is moved onto a tax table", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await addEmployee(page, "Åsa Öberg", "19800101-1231", "35000", none(page));
  const row = page.getByRole("row", { name: /^Åsa Öberg/ });
  await expect(row).toContainText("–");

  await row.getByRole("button", { name: "Redigera" }).click();
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Skattetabell" });
  await page.getByLabel("Tabell").selectOption("33");
  await page.getByLabel("Kolumn").selectOption({ label: "1 – Lön (under 66 år)" });
  await page.getByRole("button", { name: "Spara ändringar" }).click();

  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText("Tabell 33, kol 1");
  // A setting can be changed, not removed.
  await page.getByRole("row", { name: /^Åsa Öberg/ }).getByRole("button", { name: "Redigera" }).click();
  await expect(page.getByLabel("Skatt", { exact: true }).locator("option", { hasText: "Ingen" })).toBeDisabled();
});
