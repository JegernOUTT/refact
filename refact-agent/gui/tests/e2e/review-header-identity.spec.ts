import { expect, test } from "@playwright/test";

for (const { width, height } of [
  { width: 1600, height: 900 },
  { width: 1024, height: 900 },
  { width: 768, height: 900 },
  { width: 390, height: 900 },
  { width: 390, height: 420 },
]) {
  test(`review identity and controls remain usable at ${width}x${height}`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height });
    await page.goto("/tests/e2e/review-canvas.html?tour=rich");
    const draft = page.getByRole("textbox", { name: "Chat composer" });
    await draft.fill("Keep this unsent draft");
    if (width <= 680) {
      await page
        .getByRole("radio", { name: "Canvas", exact: true })
        .locator("..")
        .click();
    }
    const canvas = page.getByRole("region", { name: "Review canvas" });
    const header = canvas.locator(":scope > header");
    const identity = header.locator('[title="feature/parser"]').first();
    await expect(identity).toHaveText(
      "Review: parser trimming and total() bounds",
    );
    const geometry = await header.evaluate((element) => {
      const identity = element.querySelector<HTMLElement>(
        '[title="feature/parser"]',
      )!.parentElement!;
      const box = element.getBoundingClientRect();
      return {
        height: box.height,
        identityWidth: identity.getBoundingClientRect().width,
        identityHeight: identity.getBoundingClientRect().height,
        whiteSpace: getComputedStyle(identity).whiteSpace,
        documentWidth: document.documentElement.scrollWidth,
        viewportWidth: document.documentElement.clientWidth,
        overflowing: Array.from(
          document.querySelectorAll<HTMLElement>("body *"),
        )
          .filter(
            (node) =>
              !element.contains(node) &&
              node.getBoundingClientRect().right > innerWidth + 1,
          )
          .slice(0, 20)
          .map((node) => ({
            tag: node.tagName,
            className: String(node.className),
            right: node.getBoundingClientRect().right,
            inHeader: element.contains(node),
          })),
      };
    });
    console.log(JSON.stringify(geometry));
    expect(geometry.height).toBeLessThanOrEqual(72);
    expect(geometry.identityWidth).toBeGreaterThan(0);
    expect(geometry.identityHeight).toBeLessThan(40);
    expect(geometry.whiteSpace).toBe("nowrap");
    await testInfo.attach("header-layout", {
      body: JSON.stringify(geometry, null, 2),
      contentType: "application/json",
    });
    await testInfo.attach("review-layout", {
      body: await page.screenshot(),
      contentType: "image/png",
    });

    // Traverse from the beginning, rather than focusing the last control directly:
    // native focus must scroll the header to reveal its rightmost enabled control.
    const controls = header.locator(
      "button:not([disabled]), select:not([disabled]), input:not([disabled])",
    );
    const last = controls.last();
    await expect(last).toBeVisible();
    await page.evaluate(
      () => (document.activeElement as HTMLElement | null)?.blur(),
    );
    let reached = false;
    for (let i = 0; i < 80; i++) {
      await page.keyboard.press("Tab");
      if (await last.evaluate((node) => node === document.activeElement)) {
        reached = true;
        break;
      }
    }
    expect(reached, "rightmost header control is reachable by Tab").toBe(true);
    await expect(last).toBeInViewport();
    const visibility = await last.evaluate((node) => {
      const control = node.getBoundingClientRect();
      const strip = node.closest("header")!.getBoundingClientRect();
      return control.left >= strip.left - 1 && control.right <= strip.right + 1;
    });
    expect(visibility).toBe(true);
    if (width <= 680) {
      await page
        .getByRole("radio", { name: "Chat", exact: true })
        .locator("..")
        .click();
    }
    await expect(draft).toHaveValue("Keep this unsent draft");
    expect(geometry.documentWidth).toBeLessThanOrEqual(geometry.viewportWidth);
  });
}
