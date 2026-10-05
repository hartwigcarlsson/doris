import { test as base, expect, type BrowserContext, type CDPSession, type Locator, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// DORIS_BIN (relative to e2e/) selects another build, e.g. the release binary.
const binary = resolve(__dirname, "..", process.env.DORIS_BIN ?? "../target/debug/doris");

async function freePort(): Promise<number> {
  return new Promise((done) => {
    const server = createServer().listen(0, () => {
      const { port } = server.address() as { port: number };
      server.close(() => done(port));
    });
  });
}

async function waitUntilUp(url: string) {
  for (let i = 0; i < 100; i++) {
    try {
      await fetch(url);
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error(`server at ${url} did not start`);
}

const devtools = new WeakMap<Page, CDPSession>();

async function cdpFor(page: Page): Promise<CDPSession> {
  let cdp = devtools.get(page);
  if (!cdp) {
    cdp = await page.context().newCDPSession(page);
    await cdp.send("WebAuthn.enable");
    devtools.set(page, cdp);
  }
  return cdp;
}

/** Gives the page a virtual passkey authenticator (Chrome DevTools WebAuthn),
 * like a laptop's or phone's built-in one. Returns its id. */
export async function addAuthenticator(page: Page): Promise<string> {
  const cdp = await cdpFor(page);
  const { authenticatorId } = await cdp.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });
  return authenticatorId;
}

export async function removeAuthenticator(page: Page, authenticatorId: string) {
  await (await cdpFor(page)).send("WebAuthn.removeVirtualAuthenticator", { authenticatorId });
}

type Fixtures = {
  /** The id of the page's own virtual authenticator. */
  authenticator: string;
  /** Base URL of a fresh server with an empty database. */
  app: string;
  /** Opens a new browser context (a second person) with its own authenticator. */
  newPerson: () => Promise<Page>;
};

export const test = base.extend<Fixtures>({
  app: async ({}, use) => {
    const port = await freePort();
    const dir = mkdtempSync(join(tmpdir(), "doris-e2e-"));
    const origin = `http://localhost:${port}`;
    const server: ChildProcess = spawn(binary, [], {
      env: {
        ...process.env,
        DORIS_DATABASE: `sqlite://${join(dir, "doris.db")}`,
        DORIS_LISTEN: `127.0.0.1:${port}`,
        DORIS_RP_ID: "localhost",
        DORIS_RP_ORIGIN: origin,
        DORIS_TAX_TABLES_URL: "http://127.0.0.1:9/rowstore",
      },
      stdio: "inherit",
    });
    try {
      await waitUntilUp(origin);
      await use(origin);
    } finally {
      server.kill();
      rmSync(dir, { recursive: true, force: true });
    }
  },
  newPerson: async ({ browser }, use) => {
    const contexts: BrowserContext[] = [];
    await use(async () => {
      const context = await browser.newContext();
      contexts.push(context);
      const page = await context.newPage();
      await addAuthenticator(page);
      return page;
    });
    await Promise.all(contexts.map((c) => c.close()));
  },
  // Every test page gets an authenticator, whether or not it asks for its id.
  authenticator: [
    async ({ page }, use) => {
      await use(await addAuthenticator(page));
    },
    { auto: true },
  ],
});

export { expect };

/** Registers through the UI and waits until signed in. */
export async function register(page: Page, app: string, opts: { email: string; name: string; passkey?: string; invitationLink?: string }) {
  await page.goto(opts.invitationLink ?? `${app}/register`);
  if (!opts.invitationLink) await page.getByLabel("E-post").fill(opts.email);
  else await expect(page.getByLabel("E-post")).toHaveValue(opts.email);
  await page.getByLabel("Namn", { exact: true }).fill(opts.name);
  await page.getByLabel("Passkeyns namn").fill(opts.passkey ?? "Laptop");
  await page.getByRole("button", { name: "Skapa konto med passkey" }).click();
  await expect(page.getByText(`Inloggad som ${opts.name}`)).toBeVisible();
}

export async function logIn(page: Page, app: string, email: string) {
  await page.goto(`${app}/login`);
  await page.getByLabel("E-post").fill(email);
  await page.getByRole("button", { name: "Logga in med passkey" }).click();
}

/** Adds a company whose first räkenskapsår starts on `start` (a 1 January), by default this year. */
export async function addCompany(
  page: Page,
  app: string,
  orgNr: string,
  name: string,
  start = `${new Date().getFullYear()}-01-01`,
  method: "Faktureringsmetoden" | "Kontantmetoden" = "Faktureringsmetoden",
) {
  await page.goto(`${app}/companies`);
  await page.getByRole("main").getByRole("link", { name: "Lägg till företag" }).click();
  await page.getByLabel("Organisationsnummer").fill(orgNr);
  await page.getByLabel("Företagsnamn").fill(name);
  await page.getByLabel("Juridisk form").selectOption({ label: "Aktiebolag" });
  await page.getByLabel("Postort").fill("Stockholm");
  await page.getByLabel("Räkenskapsåret börjar").fill(start);
  await expect(page.getByText(`Räkenskapsåret slutar ${start.slice(0, 4)}-12-31.`)).toBeVisible();
  await page.getByLabel(method).check();
  await page.getByRole("button", { name: "Spara företag" }).click();
  await expect(page.getByRole("heading", { name })).toBeVisible();
}

export async function addSupplier(page: Page, app: string, name: string) {
  await page.goto(`${app}/suppliers`);
  await page.getByRole("button", { name: "Ny leverantör" }).click();
  await page.getByLabel("Namn", { exact: true }).fill(name);
  await page.getByLabel("Bankgiro").fill("5050-1055");
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^1 ${name}`) })).toBeVisible();
}

export async function addCustomer(page: Page, app: string, name: string, terms = "30") {
  await page.goto(`${app}/customers`);
  await page.getByRole("button", { name: "Ny kund" }).click();
  await page.getByLabel("Namn", { exact: true }).fill(name);
  await page.getByLabel("Betalningsvillkor (dagar)").fill(terms);
  await page.getByRole("button", { name: "Spara" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^1 ${name}`) })).toBeVisible();
}

export type Menu = "Bokföring" | "Inköp" | "Lön" | "Konto";

const MENU_OF: Record<string, Menu | null> = {
  Översikt: null,
  Kunder: null,
  Verifikationer: "Bokföring",
  Saldobalans: "Bokföring",
  Rapporter: "Bokföring",
  Kontoplan: "Bokföring",
  Räkenskapsår: "Bokföring",
  Leverantörsfakturor: "Inköp",
  Leverantörer: "Inköp",
  Lönekörningar: "Lön",
  Anställda: "Lön",
  Företag: "Konto",
  Passkeys: "Konto",
  Inbjudningar: "Konto",
};

/** Opens one of the header's menus (if it is closed) and returns its panel. */
export async function openMenu(page: Page, menu: Menu): Promise<Locator> {
  const details = page.getByRole("banner").locator("details").filter({ has: page.locator("summary", { hasText: menu }) });
  if (!(await details.evaluate((d: HTMLDetailsElement) => d.open))) await details.locator("summary").click();
  return details.getByRole("list");
}

/** Follows a link in the header, opening the menu that holds it first. */
export async function goTo(page: Page, link: string) {
  const menu = MENU_OF[link];
  if (menu === undefined) throw new Error(`no header link called ${link}`);
  const scope = menu ? await openMenu(page, menu) : page.getByRole("banner");
  await scope.getByRole("link", { name: link, exact: true }).click();
}
