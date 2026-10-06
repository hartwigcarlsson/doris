import type { Page } from "@playwright/test";
import { addCompany, bookMany, expect, goTo, register, test } from "./fixtures";

const year = new Date().getFullYear();
// A voucher's row holds the button that unfolds it; the unfolded part is a row without one.
const expand = "td > button[aria-expanded]";
const rows = (page: Page) => page.getByRole("main").locator("tbody > tr").filter({ has: page.locator(expand) });
const numbers = async (page: Page) => (await rows(page).locator(expand).allInnerTexts()).map((t) => Number(t.trim()));

async function book(page: Page, app: string, text: string, debit: string, credit: string, kronor: string, file?: boolean) {
  await page.goto(`${app}/vouchers/new`);
  await page.getByLabel("Datum").fill(`${year}-01-15`);
  await page.getByLabel("Text").fill(text);
  await page.getByLabel("Konto, rad 1").fill(debit);
  await page.getByLabel("Debet, rad 1").fill(kronor);
  await page.getByLabel("Konto, rad 2").fill(credit);
  await page.getByLabel("Kredit, rad 2").fill(kronor);
  if (file) await page.getByLabel("Underlag").setInputFiles([{ name: "kvitto.pdf", mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4\n%%EOF\n") }]);
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("status")).toBeVisible();
}

test("vouchers are listed newest first and found by text, account and amount", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Försäljning Nordvik", "1930", "3001", "1250", true);
  await book(page, app, "Hyra januari", "5010", "1930", "8000", true);
  await book(page, app, "Bankavgift", "6570", "1930", "12,50");
  await goTo(page, "Verifikationer");
  await expect.poll(() => numbers(page)).toEqual([3, 2, 1]);
  await expect(page.getByRole("status")).toHaveText("Visar 3 av 3");

  const search = page.getByLabel("Sök bland verifikationer");
  await search.fill("nordvik");
  await expect.poll(() => numbers(page)).toEqual([1]);
  await expect(page.getByRole("status")).toHaveText("Visar 1 av 1");
  await search.fill("5010");
  await expect.poll(() => numbers(page)).toEqual([2]);
  await search.fill("12,50");
  await expect.poll(() => numbers(page)).toEqual([3]);
  await search.fill("1930 hyra");
  await expect.poll(() => numbers(page)).toEqual([2]);
  await search.fill("finns inte");
  await expect(page.getByRole("main")).toContainText("Inga verifikationer matchar.");
  await search.fill("");
  await expect.poll(() => numbers(page)).toEqual([3, 2, 1]);
});

test("the filters keep vouchers without underlag, and corrections", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Med underlag", "1930", "3001", "100", true);
  await book(page, app, "Utan underlag", "1930", "3001", "200");
  await goTo(page, "Verifikationer");
  await rows(page).filter({ hasText: "Med underlag" }).getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await expect(rows(page)).toHaveCount(3);

  await page.getByLabel("Saknar underlag").check();
  await expect.poll(() => numbers(page)).toEqual([3, 2]);
  await page.getByLabel("Rättelser").check();
  await expect.poll(() => numbers(page)).toEqual([3]);
  await page.getByLabel("Saknar underlag").uncheck();
  await expect.poll(() => numbers(page)).toEqual([3, 1]);
});

test("the list shows fifty at a time", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  const id = await addCompany(page, app, "5560160680", "Exempel AB");
  await bookMany(page, app, id, 51, `${year}-01-15`);
  await goTo(page, "Verifikationer");
  await expect(page.getByRole("status")).toHaveText("Visar 50 av 51");
  await expect(rows(page)).toHaveCount(50);
  await expect.poll(async () => (await numbers(page))[0]).toBe(51);
  // A search starts from the top again, and an expanded row is the row shown.
  await page.getByRole("button", { name: "Visa fler" }).click();
  await expect(rows(page)).toHaveCount(51);
  await expect(page.getByRole("button", { name: "Visa fler" })).toHaveCount(0);
  await page.getByLabel("Sök bland verifikationer").fill("serie 51");
  await expect.poll(() => numbers(page)).toEqual([51]);
  await rows(page).locator(expand).click();
  await expect(page.getByRole("main").getByRole("row", { name: /^Summa 51,00 51,00$/ })).toBeVisible();
  await page.getByLabel("Sök bland verifikationer").fill("");
  await expect(page.getByRole("status")).toHaveText("Visar 50 av 51");
});

test("an expanded voucher says who booked it and when, and links its accounts", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna Lind" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Försäljning", "1930", "3001", "1250");
  await goTo(page, "Verifikationer");
  await rows(page).locator(expand).click();
  const main = page.getByRole("main");
  await expect(main.getByRole("heading", { name: "Behandlingshistorik" })).toBeVisible();
  // Today, in the browser's zone, to the minute.
  const now = new Date();
  const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  await expect(main.getByText(new RegExp(`^Bokförd ${today} \\d\\d:\\d\\d av Anna Lind$`))).toBeVisible();
  await main.getByRole("link", { name: /^1930 / }).click();
  await expect(page).toHaveURL(new RegExp(`/trial-balance/1930\\?fy=${year}-01-01$`));
  await expect(page.getByRole("heading", { level: 1, name: /^1930 / })).toBeVisible();
});

test("an underlag added in the row is still there after the row was filtered away", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Bankavgift", "6570", "1930", "150");
  await book(page, app, "Hyra", "5010", "1930", "8000", true);
  await goTo(page, "Verifikationer");
  const main = page.getByRole("main");
  await page.getByLabel("Saknar underlag").check();
  await expect.poll(() => numbers(page)).toEqual([1]);
  await rows(page).locator(expand).click();
  await page.getByLabel("Lägg till underlag till ver 1").setInputFiles([{ name: "avi.pdf", mimeType: "application/pdf", buffer: Buffer.from("%PDF-1.4\n%%EOF\n") }]);
  // It has its underlag now, so the filter lets it go.
  await expect(main).toContainText("Inga verifikationer matchar.");
  await page.getByLabel("Saknar underlag").uncheck();
  await rows(page).filter({ hasText: "Bankavgift" }).locator(expand).click();
  await expect(main.getByRole("button", { name: /^avi\.pdf/ })).toBeVisible();
});

test("leaving while a correction is being recorded breaks nothing", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await book(page, app, "Försäljning", "1930", "3001", "100");
  await goTo(page, "Verifikationer");
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/CorrectVoucher", async (route) => {
    await held;
    await route.continue();
  });
  await rows(page).getByRole("button", { name: "Rätta" }).click();
  await page.getByRole("button", { name: "Bekräfta rättelse" }).click();
  await goTo(page, "Kontoplan");
  await expect(page.getByRole("heading", { level: 1, name: "Kontoplan" })).toBeVisible();
  release();
  await page.waitForTimeout(500);
  await goTo(page, "Verifikationer");
  await expect.poll(() => numbers(page)).toEqual([2, 1]);
  expect(errors).toEqual([]);
});

test.describe("in a zone with summer time", () => {
  test.use({ timezoneId: "Europe/Stockholm" });

  test("a voucher's time is shown as it was then, not shifted by today's summer time", async ({ page, app }) => {
    await register(page, app, { email: "anna@example.se", name: "Anna" });
    await addCompany(page, app, "5560160680", "Exempel AB");
    const local = (d: Date) => d.toLocaleString("sv-SE", { timeZone: "Europe/Stockholm", hour: "2-digit", minute: "2-digit" });
    const before = new Date();
    await book(page, app, "Försäljning", "1930", "3001", "100");
    const after = new Date();
    // Look at it half a year from now, on the other side of the clock change.
    const summer = /\+02|GMT\+2/.test(before.toLocaleString("en-GB", { timeZone: "Europe/Stockholm", timeZoneName: "shortOffset" }));
    await page.clock.setFixedTime(new Date(`${before.getFullYear() + 1}-${summer ? "01" : "07"}-15T12:00:00Z`));
    await goTo(page, "Verifikationer");
    await rows(page).locator(expand).click();
    const line = await page.getByRole("main").getByText(/^Bokförd /).innerText();
    expect([local(before), local(after)]).toContain(line.match(/ (\d\d:\d\d) /)![1]);
  });
});
