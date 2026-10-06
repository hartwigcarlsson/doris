import { request } from "@playwright/test";
import { test, expect, register, addCompany, goTo } from "./fixtures";

/** ListCompanies (an empty message) over gRPC-Web with only a bearer token:
 * no browser, no cookie, as doris-cli calls. Returns the grpc-status. */
async function listCompaniesWith(app: string, token: string): Promise<string | undefined> {
  const cli = await request.newContext();
  const response = await cli.post(`${app}/doris.company.v1.CompanyService/ListCompanies`, {
    headers: { "content-type": "application/grpc-web+proto", "x-grpc-web": "1", authorization: `Bearer ${token}` },
    data: Buffer.from([0, 0, 0, 0, 0]),
  });
  const status = response.headers()["grpc-status"] ?? (await response.body()).toString("latin1").match(/grpc-status: ?(\d+)/)?.[1];
  await cli.dispose();
  return status;
}

test("a token is created, shown once, used without a session and revoked", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");

  await goTo(page, "API-tokens");
  await expect(page.getByRole("heading", { level: 1, name: "API-tokens" })).toBeVisible();
  await page.getByRole("link", { name: "Ny token" }).click();
  await page.getByLabel("Namn").fill("doris-cli");
  await page.getByRole("group", { name: "Exempel AB" }).getByLabel("Läsa bokföring").check();
  await page.getByRole("button", { name: "Skapa token" }).click();

  const secret = await page.getByLabel("Token").inputValue();
  expect(secret).toMatch(/^doris_[A-Za-z0-9_-]{43}$/);
  expect(await listCompaniesWith(app, secret)).toBe("0");

  await page.getByRole("link", { name: "Klar" }).click();
  const row = page.getByRole("row", { name: /doris-cli/ });
  await expect(row.getByText("Aktiv")).toBeVisible();
  await expect(row.getByText("Exempel AB")).toBeVisible();
  page.once("dialog", (dialog) => dialog.accept());
  await row.getByRole("button", { name: "Återkalla" }).click();
  await expect(row.getByText("Återkallad")).toBeVisible();

  expect(await listCompaniesWith(app, secret)).toBe("16");
});

test("a token without any scope is refused in Swedish", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  await page.goto(`${app}/settings/tokens/new`);
  await page.getByLabel("Namn").fill("doris-cli");
  await page.getByRole("button", { name: "Skapa token" }).click();
  await expect(page.getByRole("alert")).toHaveText("Ge token minst en behörighet.");
});
