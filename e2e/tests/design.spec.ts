import { addCompany, expect, register, test } from "./fixtures";

// Spacing measured from the shadcn preset b1Gdz9bFY reference (mira): cards
// ~336px wide, 16px between fields, 8px from label to input, 16px from the
// last field to the submit button.
test("forms follow the preset's spacing", async ({ page, app }) => {
  await page.goto(`${app}/register`);
  await page.getByLabel("Passkeyns namn").waitFor();

  const m = await page.evaluate(() => {
    const box = (el: Element) => el.getBoundingClientRect();
    const labels = [...document.querySelectorAll("form label")];
    const inputs = [...document.querySelectorAll("form input")];
    const button = document.querySelector("form button")!;
    return {
      cardWidth: box(document.querySelector("section")!).width,
      labelToInput: box(inputs[0]).top - box(labels[0]).bottom,
      betweenFields: box(labels[1]).top - box(inputs[0]).bottom,
      lastFieldToButton: box(button).top - box(inputs[inputs.length - 1]).bottom,
    };
  });

  expect(m.labelToInput).toBe(8);
  expect(m.betweenFields).toBe(16);
  expect(m.lastFieldToButton).toBe(16);
  expect(m.cardWidth).toBeLessThanOrEqual(352);
});

// An admin with a company sees every header link; the picker must keep its
// width and nothing may spill out of the header, on a desktop or a phone.
test("the header keeps the company picker readable", async ({ page, app }) => {
  await register(page, app, { email: "anna@example.se", name: "Anna" });
  await addCompany(page, app, "5560160680", "Exempel AB");
  const banner = page.getByRole("banner");
  await expect(banner.getByRole("link", { name: "Rapporter" })).toBeVisible();

  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 800 });
    const picker = await page.getByLabel("Aktivt företag").boundingBox();
    expect(picker!.width, `picker at ${width}px`).toBeGreaterThanOrEqual(150);
    const spills = await banner.evaluate((header) =>
      [...header.querySelectorAll("nav")].some((nav) => nav.scrollWidth > nav.clientWidth),
    );
    expect(spills, `header spills at ${width}px`).toBe(false);
  }
});
