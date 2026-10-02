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

async function bookSale(page: import("@playwright/test").Page, text: string, kronor: string) {
  await page.getByRole("link", { name: "Ny verifikation" }).click();
  await page.getByLabel("Text").fill(text);
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill("3001 Försäljning inom Sverige, 25 % moms");
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  await page.getByRole("button", { name: "Bokför" }).click();
}

test("a voucher is booked, listed and corrected", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();

  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await expect(page.getByLabel("Text")).toHaveValue("");

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  const first = page.getByRole("row", { name: /^1 / });
  await expect(first).toContainText("Försäljning kassa");
  await expect(first).toContainText("1 250,00");
  await first.getByRole("button", { name: "1" }).click();
  await expect(page.getByText("3001 Försäljning inom Sverige, 25 % moms")).toBeVisible();

  await first.getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(page.getByRole("row", { name: /^1 / })).toContainText("Rättad av ver 2");
  await expect(page.getByRole("row", { name: /^2 / })).toContainText("Rättelse av ver 1");
  await expect(page.getByRole("row", { name: /^1 / }).getByRole("button", { name: "Rätta" })).toHaveCount(0);
});

test("an unbalanced voucher is refused in Swedish and nothing is booked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers/new`);

  await page.getByLabel("Text").fill("Fel");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("100");
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill("99");
  await expect(page.getByText("Differens 1,00")).toBeVisible();
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("alert")).toHaveText("Debet och kredit måste vara lika stora.");

  await page.getByLabel("Kredit, rad 2").fill("1oo");
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("alert")).toHaveText("Skriv beloppen som 1 234,50.");

  await page.goto(`${app}/vouchers`);
  await expect(page.getByRole("row", { name: /^1 / })).toHaveCount(0);
});

test("an added account can be used in a voucher", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/accounts`);
  await page.getByLabel("Nummer").fill("1931");
  await page.getByLabel("Namn").fill("Sparkonto");
  await page.getByRole("button", { name: "Lägg till konto" }).click();
  await expect(page.getByRole("row", { name: /^1931 Sparkonto/ })).toBeVisible();

  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Text").fill("Överföring");
  await page.getByLabel("Konto, rad 1").fill("1931 Sparkonto");
  await page.getByLabel("Debet, rad 1").fill("500");
  await page.getByLabel("Konto, rad 2").fill("1930");
  await page.getByLabel("Kredit, rad 2").fill("500");
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
});

test("voucher line labels renumber after a row is removed", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers/new`);

  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByRole("button", { name: "Ta bort" }).first().click();
  await page.getByRole("button", { name: "Lägg till rad" }).click();
  for (const field of ["Konto", "Debet", "Kredit"]) {
    await expect(page.getByLabel(`${field}, rad 1`)).toHaveCount(1);
    await expect(page.getByLabel(`${field}, rad 2`)).toHaveCount(1);
  }
  await expect(page.getByLabel("Konto, rad 1")).toHaveValue("");
});

test("switching company clears the forms so nothing is booked in the wrong company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  const company = page.getByLabel("Aktivt företag");
  await expect(company.locator("option:checked")).toHaveText("Exempel AB");

  await page.goto(`${app}/accounts`);
  await page.getByLabel("Nummer").fill("1931");
  await page.getByLabel("Namn").fill("Sparkonto");
  await company.selectOption({ label: "Bolaget AB" });
  await expect(page.getByLabel("Nummer")).toHaveValue("");
  await expect(page.getByLabel("Namn")).toHaveValue("");

  await company.selectOption({ label: "Exempel AB" });
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Text").fill("Försäljning");
  await page.getByLabel("Konto, rad 1").fill("1930");
  await page.getByLabel("Debet, rad 1").fill("100");
  await page.getByLabel("Konto, rad 2").fill("3001");
  await page.getByLabel("Kredit, rad 2").fill("100");
  await company.selectOption({ label: "Bolaget AB" });

  await expect(page.getByLabel("Text")).toHaveValue("");
  for (const field of ["Konto", "Debet", "Kredit"]) {
    await expect(page.getByLabel(`${field}, rad 1`)).toHaveValue("");
    await expect(page.getByLabel(`${field}, rad 2`)).toHaveValue("");
  }
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("alert")).toBeVisible();

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await expect(company.locator("option:checked")).toHaveText("Bolaget AB");
  await expect(page.getByLabel("Räkenskapsår")).not.toHaveValue("");
  await expect(page.getByRole("row", { name: /^1 / })).toHaveCount(0);
});

test("choosing another fiscal year clears an old error", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.goto(`${app}/companies`);
  await page.getByRole("main").getByRole("link", { name: "Lägg till företag" }).click();
  await page.getByLabel("Organisationsnummer").fill("5560160680");
  await page.getByLabel("Företagsnamn").fill("Exempel AB");
  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Postort").fill("Stockholm");
  await page.getByLabel("Räkenskapsåret börjar").fill("2025-01-01");
  await page.getByLabel("Faktureringsmetoden").check();
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("heading", { name: "Exempel AB" })).toBeVisible();
  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await bookSale(page, "Försäljning", "100");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  const first = page.getByRole("row", { name: /^1 / });
  await first.getByRole("button", { name: "Rätta" }).click();
  await page.getByLabel("Datum för rättelse av ver 1").fill("2025-06-01");
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(page.getByRole("alert")).toBeVisible();

  await page.getByLabel("Räkenskapsår").selectOption({ label: "2025-01-01 – 2025-12-31" });
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("the grundbok and the chart fit without scrolling sideways", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.getByRole("banner").getByRole("link", { name: "Verifikationer" }).click();
  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  for (const path of ["/vouchers", "/accounts"]) {
    await page.goto(`${app}${path}`);
    const table = page.getByRole("table");
    await expect(table.getByRole("row").nth(1)).toBeVisible();
    const overflow = await table.evaluate((t) => t.scrollWidth - t.parentElement!.clientWidth);
    expect(overflow, `${path} table overflows its container`).toBeLessThanOrEqual(0);
  }
  await expect(page.getByRole("columnheader", { name: "Konto" })).toBeInViewport();
});

test("the trial balance and an account's ledger show what was booked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers`);
  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");

  await page.getByRole("banner").getByRole("link", { name: "Saldobalans" }).click();
  await expect(page.getByRole("heading", { name: "Saldobalans" })).toBeVisible();
  await expect(page.getByRole("row", { name: /^1930 Företagskonto/ })).toContainText("1 250,00");
  await expect(page.getByRole("row", { name: /^3001 / })).toContainText("-1 250,00");
  await expect(page.getByText("Beräknat resultat 1 250,00")).toBeVisible();
  await expect(page.getByText("Summa saldo 0,00")).toBeVisible();
  // The company's first year has no year before it to be preliminary about.
  await expect(page.getByText(/preliminära/)).toHaveCount(0);

  await page.getByRole("link", { name: "1930", exact: true }).click();
  await expect(page.getByRole("heading", { name: "1930 Företagskonto/checkkonto/affärskonto" })).toBeVisible();
  await expect(page.getByRole("row", { name: /Försäljning kassa/ })).toContainText("1 250,00");

  await page.getByRole("link", { name: "Tillbaka till saldobalansen" }).click();
  await expect(page.getByRole("heading", { name: "Saldobalans" })).toBeVisible();
});

test("the trial balance follows the active company", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/vouchers`);
  await bookSale(page, "Försäljning kassa", "1250");
  await expect(page.getByRole("status")).toHaveText("Verifikation 1 bokförd");
  await page.getByRole("banner").getByRole("link", { name: "Saldobalans" }).click();
  await expect(page.getByRole("row", { name: /^1930 / })).toBeVisible();

  await page.getByLabel("Aktivt företag").selectOption({ label: "Bolaget AB" });

  await expect(page.getByText("Inga verifikationer under räkenskapsåret.")).toBeVisible();
  await expect(page.getByRole("row", { name: /^1930 / })).toHaveCount(0);
});

test("a junk account in the URL shows a Swedish error", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await page.goto(`${app}/trial-balance/abc?fy=nonsense`);

  await expect(page.getByRole("alert")).toHaveText("Kontonumret ska vara fyra siffror, 1000–8999.");
});

test("the chosen fiscal year stays in the URL and survives a reload", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  // A company whose first räkenskapsår is 2025, so it has two years by now.
  await addCompany(page, app, "5560160680", "Exempel AB", "2025-01-01");

  await page.getByRole("banner").getByRole("link", { name: "Saldobalans" }).click();
  // The newest year is the second, and the first is still open.
  await expect(page.getByText(/preliminära/)).toBeVisible();
  await page.getByLabel("Räkenskapsår").selectOption("2025-01-01");
  await expect(page.getByText(/preliminära/)).toHaveCount(0);
  await expect(page).toHaveURL(/[?&]fy=2025-01-01/);

  await page.reload();

  await expect(page.getByLabel("Räkenskapsår")).toHaveValue("2025-01-01");
});
