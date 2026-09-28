import { expect, test } from "./fixtures";

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
