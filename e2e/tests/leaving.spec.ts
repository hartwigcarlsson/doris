import { addCompany, expect, register, test } from "./fixtures";

// A page asks the server for its data when it opens. If the user has gone
// elsewhere when the answer comes, the page's own state is gone too, and
// touching it must not bring the app down (the release wasm aborts on a panic).
const pages = [
  "/", "/companies", "/companies/new", "/accounts", "/vouchers", "/vouchers/new", "/customers", "/suppliers",
  "/customer-invoices", "/customer-invoices/new", "/supplier-invoices", "/supplier-invoices/new", "/agi",
  "/trial-balance", "/trial-balance/1930", "/financial-statements", "/fiscal-years", "/vat", "/vat/202603", "/opening-balances",
  "/employees", "/payroll-runs", "/payroll-runs/new", "/settings/passkeys", "/admin/invitations",
];

// A page often loads in steps (the years, then the year's figures), so each
// page is left at every step: with nothing answered yet, after the first
// answer, and after the second.
for (const path of pages) for (const answered of [0, 1, 2]) {
  test(`leaving ${path} after ${answered} answers breaks nothing`, async ({ page, app }) => {
    await register(page, app, { email: "anna@example.se", name: "Anna" });
    await addCompany(page, app, "5560160680", "Exempel AB");
    await page.goto(`${app}/settings/passkeys`);
    await expect(page.getByRole("heading", { level: 1, name: "Passkeys" })).toBeVisible();
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
    page.on("console", (m) => m.type() === "error" && errors.push(m.text().split("\n").slice(0, 2).join(" ")));

    // Let the first `answered` calls through and hold the rest.
    let release = () => {};
    const held = new Promise<void>((r) => (release = r));
    let calls = 0;
    await page.route("**/doris.*/**", async (route) => {
      calls += 1;
      if (calls > answered) await held;
      await route.continue();
    });
    const go = (to: string) =>
      page.evaluate((to) => {
        history.pushState({}, "", to);
        dispatchEvent(new PopStateEvent("popstate"));
      }, to);
    await go(path);
    await expect(page).toHaveURL(new RegExp(`${path.replace(/[/]/g, "\\/")}(\\?.*)?$`));
    await page.waitForTimeout(300);
    const asked = calls;
    // Leave before anything has answered, then let the answers come.
    await go("/companies/new");
    await expect(page.getByRole("heading", { level: 1, name: "Lägg till företag" })).toBeVisible();
    release();
    await page.waitForTimeout(700);
    expect(errors, `${asked} calls were made before leaving`).toEqual([]);
    // The app is still alive.
    await page.unrouteAll({ behavior: "ignoreErrors" });
    await go("/settings/passkeys");
    await expect(page.getByRole("heading", { level: 1, name: "Passkeys" })).toBeVisible();
  });
}

// The same for something the user started: the answer to a save or a change
// may come after they have left the page.
type Step = (page: import("@playwright/test").Page) => Promise<void>;
// What it is, the call to hold, how to start it, and how to see afterwards that the server did it.
const actions: [string, string, Step, Step][] = [
  [
    "an account being deactivated",
    "SetAccountActive",
    async (page) => {
      await page.goto("/accounts");
      await page.getByRole("row", { name: /^1930 / }).getByRole("button", { name: "Inaktivera" }).click();
    },
    async (page) => {
      await page.goto("/accounts");
      await page.getByLabel("Visa inaktiva").check();
      await expect(page.getByRole("row", { name: /^1930 / })).toContainText("Inaktivt");
    },
  ],
  [
    "a customer being saved",
    "AddCustomer",
    async (page) => {
      await page.goto("/customers");
      await page.getByRole("button", { name: "Ny kund" }).click();
      await page.getByLabel("Namn", { exact: true }).fill("Kund AB");
      await page.getByRole("button", { name: "Spara" }).click();
    },
    async (page) => {
      await page.goto("/customers");
      await expect(page.getByRole("row", { name: /^1 Kund AB/ })).toBeVisible();
    },
  ],
  [
    "a voucher being booked",
    "RecordVoucher",
    async (page) => {
      await page.goto("/vouchers/new");
      await page.getByLabel("Datum").fill(`${new Date().getFullYear()}-01-15`);
      await page.getByLabel("Text").fill("Försäljning");
      await page.getByLabel("Konto, rad 1").fill("1930");
      await page.getByLabel("Debet, rad 1").fill("100");
      await page.getByLabel("Konto, rad 2").fill("3001");
      await page.getByLabel("Kredit, rad 2").fill("100");
      await page.getByRole("button", { name: "Bokför" }).click();
    },
    async (page) => {
      await page.goto("/vouchers");
      await expect(page.getByRole("row", { name: /^1 .* Försäljning 100,00/ })).toBeVisible();
    },
  ],
];

for (const [what, rpc, start, done] of actions) {
  test(`leaving with ${what} breaks nothing, and it is still done`, async ({ page, app }) => {
    await register(page, app, { email: "anna@example.se", name: "Anna" });
    await addCompany(page, app, "5560160680", "Exempel AB");
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(String(e).split("\n")[0]));
    page.on("console", (m) => m.type() === "error" && errors.push(m.text().split("\n").slice(0, 2).join(" ")));
    let release = () => {};
    const held = new Promise<void>((r) => (release = r));
    let asked = false;
    await page.route(`**/${rpc}`, async (route) => {
      asked = true;
      await held;
      await route.continue();
    });
    const go = (to: string) =>
      page.evaluate((to) => {
        history.pushState({}, "", to);
        dispatchEvent(new PopStateEvent("popstate"));
      }, to);
    // `start` uses paths relative to the app.
    const goto = page.goto.bind(page);
    page.goto = ((url: string, options?: object) => goto(url.startsWith("/") ? `${app}${url}` : url, options)) as typeof page.goto;
    await start(page);
    await expect.poll(() => asked).toBe(true);
    await go("/settings/passkeys");
    await expect(page.getByRole("heading", { level: 1, name: "Passkeys" })).toBeVisible();
    const answered = page.waitForResponse((r) => r.url().endsWith(`/${rpc}`));
    release();
    await answered;
    await page.waitForTimeout(500);
    expect(errors).toEqual([]);
    // The app is alive, and the server did what was asked.
    await page.unrouteAll({ behavior: "ignoreErrors" });
    await go("/accounts");
    await expect(page.getByRole("heading", { level: 1, name: "Kontoplan" })).toBeVisible();
    await done(page);
  });
}
