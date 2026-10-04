import { addCompany, expect, register, test } from "./fixtures";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const MONTHS = ["januari", "februari", "mars", "april", "maj", "juni", "juli", "augusti", "september", "oktober", "november", "december"];
const pad = (n: number) => String(n).padStart(2, "0");
const now = new Date();
const today = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
const period = `${now.getFullYear()}${pad(now.getMonth() + 1)}`;
const periodLabel = `${MONTHS[now.getMonth()]} ${now.getFullYear()}`;

test("a month is declared, downloaded and corrected", async ({ page, app }, testInfo) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const banner = (name: string) => page.getByRole("banner").getByRole("link", { name });

  await banner("Anställda").click();
  await page.getByLabel("Namn").fill("Åsa Öberg");
  await page.getByLabel("Personnummer").fill("19800101-1231");
  await page.getByLabel("Månadslön (kr)").fill("35000");
  await page.getByLabel("Skatt", { exact: true }).selectOption({ label: "Ingen (skatten skrivs in för hand)" });
  await page.getByRole("button", { name: "Lägg till anställd" }).click();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toBeVisible();

  await banner("Lönekörningar").click();
  await page.getByRole("link", { name: "Ny lönekörning" }).click();
  await expect(page.getByLabel("Brutto, Åsa Öberg")).toHaveValue(/35\s000,00/);
  await page.getByLabel("Utbetalningsdag").fill(today);
  await page.getByLabel("Skatt, Åsa Öberg").fill("8000");
  await page.getByRole("button", { name: "Färdigställ" }).click();
  await page.getByRole("button", { name: "Bokför" }).click();
  await expect(page.getByRole("link", { name: "Ver 1" })).toBeVisible();
  const runUrl = page.url();

  await banner("Arbetsgivardeklaration").click();
  const row = page.getByRole("row", { name: new RegExp(`^${periodLabel}`) });
  await expect(row).toContainText("Ej deklarerad");
  await expect(row).toContainText(/10\s997,00/);
  await expect(page.getByLabel("Namn")).toHaveValue("Anna");
  await expect(page.getByLabel("E-post")).toHaveValue("anna@example.se");
  await page.getByLabel("Telefon").fill("070-123 45 67");
  await page.getByRole("button", { name: "Spara kontaktperson" }).click();

  await row.getByRole("button", { name: periodLabel }).click();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText("Ny");
  const [first] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "Ladda ner fil" }).click()]);
  expect(first.suggestedFilename()).toBe(`AGI_165560160680_${period}.xml`);
  const firstPath = join(testInfo.outputDir, `first-${first.suggestedFilename()}`);
  await first.saveAs(firstPath);
  const xml = readFileSync(firstPath, "utf8");
  expect(xml).toContain('<agd:BetalningsmottagarId faltkod="215">198001011231</agd:BetalningsmottagarId>');
  expect(xml).toContain('<agd:SummaArbAvgSlf faltkod="487">10997</agd:SummaArbAvgSlf>');

  await page.getByRole("button", { name: "Markera som inlämnad" }).click();
  await page.getByRole("button", { name: "Bekräfta" }).click();
  await expect(page.getByRole("row", { name: new RegExp(`^${periodLabel}`) })).toContainText("Deklarerad");

  await page.goto(runUrl);
  await page.getByRole("button", { name: "Backa bokföring" }).click();
  await page.getByRole("button", { name: "Bekräfta backning" }).click();
  await expect(page.getByText("Att bokföra")).toBeVisible();

  await banner("Arbetsgivardeklaration").click();
  const changed = page.getByRole("row", { name: new RegExp(`^${periodLabel}`) });
  await expect(changed).toContainText("Ändrad");
  await changed.getByRole("button", { name: periodLabel }).click();
  await expect(page.getByRole("row", { name: /^Åsa Öberg/ })).toContainText("Borttag");
  const [second] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "Ladda ner fil" }).click()]);
  const secondPath = join(testInfo.outputDir, `second-${second.suggestedFilename()}`);
  await second.saveAs(secondPath);
  expect(readFileSync(secondPath, "utf8")).toContain('<agd:Borttag faltkod="205">1</agd:Borttag>');
  console.log(`AGI files: ${firstPath} ${secondPath}`);
});
