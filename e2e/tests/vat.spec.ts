import type { Page } from "@playwright/test";
import { addCompany, expect, goTo, register, test } from "./fixtures";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// Last year, so the fourth quarter has ended whatever day the test runs.
const last = new Date().getFullYear() - 1;

async function voucher(page: Page, app: string, date: string, text: string, lines: [string, string, string][]) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(date);
  await page.getByLabel("Text").fill(text);
  // The editor starts with two rows.
  for (let i = 2; i < lines.length; i++) await page.getByRole("button", { name: "Lägg till rad" }).click();
  for (const [i, [account, debit, credit]] of lines.entries()) {
    await page.getByLabel(`Konto, rad ${i + 1}`).fill(account);
    if (debit) await page.getByLabel(`Debet, rad ${i + 1}`).fill(debit);
    if (credit) await page.getByLabel(`Kredit, rad ${i + 1}`).fill(credit);
  }
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
}

test("a quarter is declared, downloaded, submitted and changed", async ({ page, app }, testInfo) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${last}-01-01`);
  await voucher(page, app, `${last}-11-10`, "Försäljning", [["1930", "1250", ""], ["3001", "", "1000"], ["2611", "", "250"]]);
  await voucher(page, app, `${last}-12-05`, "Inköp", [["4010", "320", ""], ["2640", "80", ""], ["1930", "", "400"]]);

  await goTo(page, "Moms");
  await expect(page.getByRole("heading", { level: 1, name: "Moms" })).toBeVisible();
  await page.goto(`${app}/vat?fy=${last}-01-01`);
  const q4 = page.getByRole("row", { name: new RegExp(`oktober–december ${last}`) });
  await expect(q4).toContainText("Att lämna");
  await expect(q4).toContainText("170");
  await q4.getByRole("link").click();
  await expect(page.getByRole("heading", { level: 1, name: `Momsdeklaration oktober–december ${last}` })).toBeVisible();
  await expect(page.getByRole("button", { name: /Momspliktig försäljning som inte ingår.*05.*1\s000/ })).toBeVisible();
  await page.getByRole("button", { name: /Ingående moms att dra av/ }).click();
  await expect(page.getByText("2640 Ingående moms")).toBeVisible();

  const [file] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "Ladda ner fil" }).click()]);
  expect(file.suggestedFilename()).toBe(`moms_5560160680_${last}12.xml`);
  const path = join(testInfo.outputDir, file.suggestedFilename());
  await file.saveAs(path);
  const xml = readFileSync(path, "utf8");
  expect(xml).toContain("<MomsUtgHog>250</MomsUtgHog>");
  expect(xml).toContain("<MomsIngAvdr>80</MomsIngAvdr>");
  expect(xml).toContain("<MomsBetala>170</MomsBetala>");

  await page.getByRole("button", { name: "Markera inlämnad…" }).click();
  await page.getByRole("button", { name: "Bekräfta" }).dblclick();
  await expect(page.getByRole("status")).toContainText("verifikation 3");
  await expect(page.getByText("Perioden är redan inlämnad och har inte ändrats.")).toHaveCount(0);

  await page.goto(`${app}/vouchers`);
  await page.getByLabel("Räkenskapsår").selectOption(`${last}-01-01`);
  await expect(page.getByText(`Momsavräkning oktober–december ${last}`)).toBeVisible();

  await voucher(page, app, `${last}-12-20`, "Sen försäljning", [["1930", "125", ""], ["3001", "", "100"], ["2611", "", "25"]]);
  await page.goto(`${app}/vat?fy=${last}-01-01`);
  await expect(page.getByRole("row", { name: new RegExp(`oktober–december ${last}`) })).toContainText("Ändrad");

  // Submitted again, the new settlement books only the late 25 kronor.
  await page.goto(`${app}/vat/${last}12`);
  await page.getByRole("button", { name: "Markera inlämnad…" }).click();
  await page.getByRole("button", { name: "Bekräfta" }).click();
  await expect(page.getByRole("status")).toContainText("verifikation 5");
  await page.goto(`${app}/vouchers`);
  await page.getByLabel("Räkenskapsår").selectOption(`${last}-01-01`);
  const settlement = page.getByRole("row", { name: new RegExp(`^5 .*Momsavräkning oktober–december ${last}`) });
  await expect(settlement).toContainText("25,00");
  await settlement.getByRole("button", { name: "5", exact: true }).click();
  await expect(page.getByRole("row", { name: /^2611 .*25,00/ })).toBeVisible();
  await expect(page.getByRole("row", { name: /^2650 .*25,00/ })).toBeVisible();
});

test("an account's momsruta changes the declaration", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB", `${last}-01-01`);
  await voucher(page, app, `${last}-11-10`, "Momsfri försäljning", [["1930", "500", ""], ["3004", "", "500"]]);
  await goTo(page, "Kontoplan");
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/SetAccountVatBox")),
    page.getByLabel("Momsruta för 3004").selectOption("5"),
  ]);
  await page.goto(`${app}/vat/${last}12`);
  await expect(page.getByRole("button", { name: /Momspliktig försäljning som inte ingår.*05.*500/ })).toBeVisible();
});
