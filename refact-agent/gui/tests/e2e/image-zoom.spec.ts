import { expect, test, type Page } from "@playwright/test";

async function anchoredWheel(page: Page, deltaY: number) {
  const viewport = page.getByTestId("image-preview-viewport");
  const box = await viewport.boundingBox();
  if (!box) throw new Error("Missing viewport");
  const pointer = { x: box.x + 390, y: box.y + 180 };
  const before = await page.getByRole("img").boundingBox();
  if (!before) throw new Error("Missing image");
  const point = {
    x: (pointer.x - before.x) / before.width,
    y: (pointer.y - before.y) / before.height,
  };
  await page.mouse.move(pointer.x, pointer.y);
  await page.mouse.wheel(0, deltaY);
  await expect
    .poll(async () => (await page.getByRole("img").boundingBox())?.width)
    .not.toBe(before.width);
  const after = await page.getByRole("img").boundingBox();
  if (!after) throw new Error("Missing zoomed image");
  const actual = {
    x: after.x + point.x * after.width,
    y: after.y + point.y * after.height,
  };
  expect(Math.abs(actual.x - pointer.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(actual.y - pointer.y)).toBeLessThanOrEqual(1);
  console.log(JSON.stringify({ deltaY, pointer, before, after, actual }));
}

for (const deviceScaleFactor of [1, 2]) {
  test.describe(`image wheel zoom DPR ${deviceScaleFactor}`, () => {
    test.use({ deviceScaleFactor, viewport: { width: 1000, height: 800 } });

    test("anchors fit-to-overflow, scrolled and rotated zoom in both directions", async ({
      page,
    }) => {
      await page.goto("/tests/e2e/image-zoom.html");
      await expect(page.getByLabel("Zoom level")).toHaveText("38%");
      await anchoredWheel(page, -100);
      await page.getByRole("button", { name: "Actual size" }).click();
      const viewport = page.getByTestId("image-preview-viewport");
      await viewport.evaluate((element) => {
        element.scrollLeft = 320;
        element.scrollTop = 240;
      });
      await anchoredWheel(page, -100);
      await anchoredWheel(page, 100);
      await page.getByRole("button", { name: "Rotate clockwise" }).click();
      await viewport.evaluate((element) => {
        element.scrollLeft = 180;
        element.scrollTop = 310;
      });
      await expect(page.getByRole("img")).toHaveAttribute(
        "data-rotation",
        "90",
      );
      await anchoredWheel(page, -100);
      await anchoredWheel(page, 100);
      await page.getByRole("button", { name: "Fit to view" }).click();
      await page.mouse.move(454, 244);
      await page.mouse.wheel(0, 100);
      await expect(page.getByLabel("Zoom level")).toHaveText("23%");
      const geometry = await viewport.evaluate((element) => {
        const view = element.getBoundingClientRect();
        const stage = element.firstElementChild!.getBoundingClientRect();
        return {
          left: element.scrollLeft,
          top: element.scrollTop,
          centerX: stage.left + stage.width / 2 - view.left,
          centerY: stage.top + stage.height / 2 - view.top,
          width: element.clientWidth,
          height: element.clientHeight,
        };
      });
      expect(geometry.left).toBe(0);
      expect(geometry.top).toBe(0);
      expect(geometry.centerX).toBeCloseTo(geometry.width / 2, 0);
      expect(geometry.centerY).toBeCloseTo(geometry.height / 2, 0);
    });
  });
}
