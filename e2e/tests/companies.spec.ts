import { addCompany, expect, goTo, register, test } from "./fixtures";
import type { Page } from "@playwright/test";


test("a user adds a company by hand and finds it in the list", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await goTo(page, "Företag");
  await expect(page.getByText("Inga företag än.")).toBeVisible();

  await addCompany(page, app, "5560160680", "Exempel AB");

  await expect(page.getByText("556016-0680")).toBeVisible();
  await expect(page.getByText("Faktureringsmetoden")).toBeVisible();
  await expect(page.getByText(/^\d{4}-01-01 – \d{4}-12-31$/)).toBeVisible();
  await goTo(page, "Företag");
  await expect(page.getByRole("link", { name: "Exempel AB" })).toBeVisible();
});

test("the form explains invalid input and a missing Bolagsverket setup in Swedish", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.goto(`${app}/companies/new`);

  // Without Bolagsverket credentials the form says so instead of offering a button.
  await expect(page.getByText("Hämtning från Bolagsverket är inte konfigurerad. Fyll i uppgifterna själv.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Hämta från Bolagsverket" })).toHaveCount(0);

  // Everything else valid, so the org nr is the error reported (the server
  // checks legal form and method before the org nr).
  await page.getByLabel("Organisationsnummer").fill("556016-0681");
  await page.getByLabel("Företagsnamn").fill("Exempel AB");
  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Faktureringsmetoden").check();
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ange ett giltigt organisationsnummer (10 siffror).");
});

test("the end of the first räkenskapsår follows from its start, unless it is shortened or extended", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await page.goto(`${app}/companies/new`);
  await page.getByLabel("Organisationsnummer").fill("5560160680");
  await page.getByLabel("Företagsnamn").fill("Exempel AB");
  await page.getByLabel("Faktureringsmetoden").check();

  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Räkenskapsåret börjar").fill("2026-07-01");
  await expect(page.getByText("Räkenskapsåret slutar 2027-06-30.")).toBeVisible();
  await page.getByLabel("Juridisk form").selectOption({ label: "Enskild firma" });
  await expect(page.getByText("Räkenskapsåret slutar 2026-12-31.")).toBeVisible();
  await expect(page.getByLabel("Räkenskapsåret slutar")).toHaveCount(0);

  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Första räkenskapsåret är förkortat eller förlängt").check();
  await expect(page.getByLabel("Räkenskapsåret slutar")).toHaveValue("2027-06-30");
  await page.getByLabel("Räkenskapsåret slutar").fill("2027-12-31"); // 18 months
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("heading", { name: "Exempel AB" })).toBeVisible();
});

test("a colleague sees a company only after being added as a member", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await goTo(page, "Inbjudningar");
  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();
  const link = await page.getByLabel("Inbjudningslänk").inputValue();
  const bo = await newPerson();
  await register(bo, app, { email: "bo@example.se", name: "Bo", invitationLink: link });

  await addCompany(page, app, "5560160680", "Exempel AB");
  const companyUrl = page.url();
  await bo.goto(companyUrl);
  await expect(bo.getByRole("alert")).toHaveText("Företaget finns inte eller så saknar du tillgång.");

  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Lägg till medlem" }).click();
  await expect(page.getByText("bo@example.se")).toBeVisible();

  await bo.goto(`${app}/companies`);
  await bo.getByRole("link", { name: "Exempel AB" }).click();
  await expect(bo.getByRole("heading", { name: "Exempel AB" })).toBeVisible();
});

test("the active company is chosen in the header and remembered", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const header = page.getByRole("banner");
  await expect(header.getByRole("link", { name: "Lägg till företag" })).toBeVisible();
  await expect(page.getByLabel("Aktivt företag")).toHaveCount(0);
  await expect(page.getByText("Du har inga företag än.")).toBeVisible();

  // Exempel AB is added last but sorts last, so only "a new company becomes
  // active" can make it the active one; the fallback would pick Bolaget AB.
  await addCompany(page, app, "5560360793", "Bolaget AB");
  await addCompany(page, app, "5560160680", "Exempel AB");
  const active = () => page.getByLabel("Aktivt företag");
  await expect(active().locator("option:checked")).toHaveText("Exempel AB");
  await page.reload();
  await expect(active().locator("option:checked")).toHaveText("Exempel AB");

  await active().selectOption({ label: "Bolaget AB" });
  await page.reload();
  await expect(active().locator("option:checked")).toHaveText("Bolaget AB");
  await active().selectOption({ label: "Exempel AB" });
  await page.goto(app);
  const card = page.getByRole("main");
  await expect(card.getByRole("heading", { name: "Aktivt företag" })).toBeVisible();
  await expect(card.getByText("Exempel AB")).toBeVisible();
  await expect(card.getByText("556016-0680")).toBeVisible();
  await card.getByRole("link", { name: "Visa företaget" }).click();
  await expect(page.getByRole("heading", { name: "Exempel AB" })).toBeVisible();
});
