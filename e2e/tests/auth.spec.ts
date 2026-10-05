import { addAuthenticator, expect, expectSignedIn, goTo, logIn, openMenu, register, removeAuthenticator, test } from "./fixtures";

test("the first user registers with a passkey and becomes admin", async ({ page, app }) => {
  await page.goto(app);
  await expect(page).toHaveURL(`${app}/register`);
  await expect(page.getByRole("heading", { name: "Skapa administratörskonto" })).toBeVisible();

  await register(page, app, { email: "anna@example.se", name: "Anna" });

  await expect((await openMenu(page, "Konto")).getByRole("link", { name: "Inbjudningar" })).toBeVisible();
});

test("a user signs out and back in with the passkey", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });

  await (await openMenu(page, "Konto")).getByRole("button", { name: "Logga ut" }).click();
  await expect(page).toHaveURL(`${app}/login`);
  await page.goto(app);
  await expect(page).toHaveURL(`${app}/login`);

  await logIn(page, app, "anna@example.se");
  await expectSignedIn(page, "Anna");
  await page.reload();
  await expectSignedIn(page, "Anna");
});

test("an unknown email fails like any other failed login", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await (await openMenu(page, "Konto")).getByRole("button", { name: "Logga ut" }).click();

  await logIn(page, app, "nobody@example.se");

  await expect(page.getByRole("alert")).toHaveText("Inloggningen misslyckades.");
});

test("an admin invites a member who registers through the link", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await goTo(page, "Inbjudningar");
  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();
  const link = await page.getByLabel("Inbjudningslänk").inputValue();
  expect(link).toContain("/register?invitation=");

  const bo = await newPerson();
  await register(bo, app, { email: "bo@example.se", name: "Bo", passkey: "Telefon", invitationLink: link });

  await expect(bo.getByRole("link", { name: "Inbjudningar" })).toHaveCount(0);
  await bo.goto(`${app}/admin/invitations`);
  await expect(bo.getByRole("alert")).toHaveText("Du saknar behörighet.");
  await page.reload();
  await expect(page.getByText("Använd", { exact: true })).toBeVisible();
});

test("registration without an invitation is closed after the first user", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });

  const stranger = await newPerson();
  await stranger.goto(`${app}/register`);

  await expect(stranger.getByText("Registrering kräver en inbjudan.")).toBeVisible();
  await expect(stranger.getByRole("button", { name: "Skapa konto med passkey" })).toHaveCount(0);
});

test("an unusable invitation shows the error and closes the registration form", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });

  const stranger = await newPerson();
  await stranger.goto(`${app}/register?invitation=bogus`);

  await expect(stranger.getByRole("alert")).toHaveText("Inbjudan finns inte eller har redan använts.");
  await expect(stranger.getByRole("button", { name: "Skapa konto med passkey" })).toHaveCount(0);
});

test("a user adds a second passkey and signs in with it", async ({ page, app, authenticator: laptop }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna", passkey: "Laptop" });
  await goTo(page, "Passkeys");
  await expect(page.getByText("Laptop")).toBeVisible();

  // Switch to another device: only the "phone" authenticator is present now.
  await removeAuthenticator(page, laptop);
  await addAuthenticator(page);
  await page.getByLabel("Passkeyns namn").fill("Telefon");
  await page.getByRole("button", { name: "Lägg till passkey" }).click();
  await expect(page.getByText("Telefon")).toBeVisible();

  await (await openMenu(page, "Konto")).getByRole("button", { name: "Logga ut" }).click();
  await logIn(page, app, "anna@example.se");
  await expectSignedIn(page, "Anna");
});

test("an invited email is shown as locked, with the reason", async ({ page, app, newPerson }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await goTo(page, "Inbjudningar");
  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();
  const link = await page.getByLabel("Inbjudningslänk").inputValue();

  const bo = await newPerson();
  await bo.goto(link);

  await expect(bo.getByLabel("E-post")).toHaveValue("bo@example.se");
  await expect(bo.getByLabel("E-post")).toHaveAttribute("readonly", "");
  await expect(bo.getByText("E-postadressen kommer från inbjudan.")).toBeVisible();

  // The dashed border marks the field as locked, also while it has focus.
  const border = () => bo.getByLabel("E-post").evaluate(async (e) => {
    await Promise.all(e.getAnimations().map((a) => a.finished));
    const s = getComputedStyle(e);
    return `${s.borderStyle} ${s.borderColor}`;
  });
  const before = await border();
  await bo.getByLabel("E-post").click();
  expect(before).toMatch(/^dashed /);
  expect(await border()).toBe(before);
});

test("a new passkey shows when it was added until it is used", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna", passkey: "MacBook" });

  await goTo(page, "Passkeys");

  await expect(page.getByText(/^Tillagd \d{4}-\d{2}-\d{2}$/)).toBeVisible();
  await expect(page.getByText("Aldrig använd")).toHaveCount(0);
});

test("form validation messages are in Swedish", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await goTo(page, "Inbjudningar");

  await page.getByLabel("E-post").fill("inte-en-epost");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();

  await expect(page.getByRole("alert")).toHaveText("Ange en giltig e-postadress.");
});

test("the invitation list only shows once there are invitations", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await goTo(page, "Inbjudningar");
  await expect(page.getByRole("heading", { name: "Bjud in" })).toBeVisible();

  await expect(page.getByRole("heading", { name: "Skickade inbjudningar" })).toHaveCount(0);

  await page.getByLabel("E-post").fill("bo@example.se");
  await page.getByRole("button", { name: "Skapa inbjudan" }).click();
  await expect(page.getByRole("heading", { name: "Skickade inbjudningar" })).toBeVisible();
});
