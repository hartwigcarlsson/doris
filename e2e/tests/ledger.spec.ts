import { addCompany, expect, register, test } from "./fixtures";

test("the chart of accounts starts from BAS and can be extended", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Kontoplan" }).click();

  const bank = page.getByRole("row", { name: /^1930 Företagskonto/ });
  await expect(bank).toContainText("Aktivt");

  await page.getByLabel("Nummer").fill("1931");
  await page.getByLabel("Namn").fill("Sparkonto");
  await page.getByRole("button", { name: "Lägg till konto" }).click();
  await expect(page.getByRole("row", { name: /^1931 Sparkonto/ })).toBeVisible();

  await page.getByLabel("Nummer").fill("1930");
  await page.getByLabel("Namn").fill("Bank");
  await page.getByRole("button", { name: "Lägg till konto" }).click();
  await expect(page.getByRole("alert")).toHaveText("Kontot finns redan i kontoplanen.");

  const sparkonto = page.getByRole("row", { name: /^1931 / });
  await sparkonto.getByRole("button", { name: "Byt namn" }).click();
  await sparkonto.getByLabel("Nytt namn för 1931").fill("Sparkonto SEB");
  await sparkonto.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("row", { name: /^1931 Sparkonto SEB/ })).toBeVisible();

  await page.getByRole("row", { name: /^1910 Kassa/ }).getByRole("button", { name: "Inaktivera" }).click();
  await expect(page.getByRole("row", { name: /^1910 Kassa/ })).toHaveCount(0);
  await page.getByLabel("Visa inaktiva").check();
  await expect(page.getByRole("row", { name: /^1910 Kassa/ })).toContainText("Inaktivt");
});

test("another tab follows the active company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  const other = await page.context().newPage();
  await other.goto(app);
  const active = (p: typeof page) => p.getByLabel("Aktivt företag").locator("option:checked");
  await expect(active(other)).toHaveText("Exempel AB");

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(active(other)).toHaveText("Bolaget AB");
});
