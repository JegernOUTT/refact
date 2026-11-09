import { expect, test, type Page } from "@playwright/test";

async function measure(page: Page) {
  return page.evaluate(() => {
    const section = document.querySelector<HTMLElement>(
      '[data-toolbar-pane-id="right"]',
    );
    const pane = document.querySelector<HTMLElement>(
      '[data-workspace-leaf-id="left"]',
    );
    const divider = document.querySelector<HTMLElement>(
      '[data-testid="workspace-vertical-divider"]',
    );
    if (!section || !pane || !divider)
      throw new Error("Missing split geometry");
    const shadow = getComputedStyle(section).boxShadow;
    const offset = Number(shadow.match(/(-?[\d.]+)px/)?.[1]);
    const sectionX = section.getBoundingClientRect().left;
    const paneX =
      pane.getBoundingClientRect().right -
      parseFloat(getComputedStyle(pane).borderRightWidth);
    const dividerRect = divider.getBoundingClientRect();
    return {
      sectionX,
      paneX,
      separatorX: sectionX + (shadow.includes("inset") ? 0 : offset),
      inset: shadow.includes("inset"),
      dividerX: dividerRect.left + dividerRect.width / 2,
    };
  });
}

for (const width of [800, 1440]) {
  for (const deviceScaleFactor of [1, 2]) {
    test(`tab separator aligns with pane border at ${width}px DPR ${deviceScaleFactor}`, async ({
      browser,
    }, testInfo) => {
      const context = await browser.newContext({
        viewport: { width, height: 700 },
        deviceScaleFactor,
      });
      const page = await context.newPage();
      await page.goto("/tests/e2e/workspace-showcase.html");
      await expect(
        page.locator('[data-toolbar-pane-id="right"]'),
      ).toBeVisible();
      const assertAligned = async () => {
        await expect
          .poll(async () => {
            const geometry = await measure(page);
            return Math.abs(geometry.separatorX - geometry.paneX);
          })
          .toBeLessThan(0.02);
        const geometry = await measure(page);
        expect(geometry.inset).toBe(false);
        expect(geometry.dividerX).toBeCloseTo(geometry.sectionX, 1);
        console.log({ width, deviceScaleFactor, ...geometry });
      };
      await assertAligned();
      const geometry = await measure(page);
      const crop = await page.screenshot({
        clip: { x: geometry.sectionX - 4, y: 0, width: 8, height: 80 },
        scale: "css",
      });
      await testInfo.attach("tab-pane-boundary", {
        body: crop,
        contentType: "image/png",
      });
      const paint = await page.evaluate(async (base64) => {
        const image = new Image();
        image.src = `data:image/png;base64,${base64}`;
        await image.decode();
        const canvas = document.createElement("canvas");
        canvas.width = image.width;
        canvas.height = image.height;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Missing screenshot canvas");
        context.drawImage(image, 0, 0);
        const pixel = (x: number, y: number) => [
          ...context.getImageData(x, y, 1, 1).data,
        ];
        return {
          top: pixel(3, 5),
          bottom: pixel(3, 60),
          bottomAdjacent: pixel(4, 60),
          adjacent: pixel(4, 5),
        };
      }, crop.toString("base64"));
      expect(paint.bottom).not.toEqual(paint.bottomAdjacent);
      expect(paint.top).not.toEqual(paint.adjacent);
      console.log({ width, deviceScaleFactor, paint });
      const divider = page.getByRole("separator", {
        name: "Resize workspace panes",
      });
      await divider.focus();
      await page.keyboard.press("ArrowRight");
      await expect(divider).toHaveAttribute("aria-valuenow", "52");
      await assertAligned();
      const bounds = await divider.boundingBox();
      if (!bounds) throw new Error("Missing divider bounds");
      await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 100);
      await page.mouse.down();
      await page.mouse.move(width * 0.6, bounds.y + 100);
      await page.mouse.up();
      await expect(divider).toHaveAttribute("aria-valuenow", "60");
      await assertAligned();
      await context.close();
    });
  }
}
