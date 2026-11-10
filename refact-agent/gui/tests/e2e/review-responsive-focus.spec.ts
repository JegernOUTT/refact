import { expect, test } from "@playwright/test";

for (const { width, height, minimumReadingHeight } of [
  { width: 390, height: 420, minimumReadingHeight: 220 },
  { width: 390, height: 844, minimumReadingHeight: 560 },
  { width: 768, height: 900, minimumReadingHeight: 640 },
  { width: 1600, height: 900, minimumReadingHeight: 780 },
]) {
  test(`review focus retains state and reading space at ${width}x${height}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height });
    await page.goto("/tests/e2e/review-canvas.html?tour=rich&title=long");
    const chat = page.getByRole("textbox", { name: "Chat composer" });
    await chat.fill("Unsent responsive draft");
    const originalChat = await chat.elementHandle();
    const canvasSwitch = page.getByRole("radio", {
      name: "Canvas",
      exact: true,
    });
    const narrow = width <= 680;
    if (narrow) {
      await expect(
        page.getByRole("radio", { name: "Chat", exact: true }),
      ).toBeChecked();
      await expect(chat).toBeFocused();
      await canvasSwitch.focus();
      await page.keyboard.press("Space");
    }
    const canvas = page.getByRole("region", { name: "Review canvas" });
    await expect(canvas).toBeVisible();
    const reader = page.locator("[data-review-document]");
    await expect(
      page.getByRole("heading", {
        name: "One operator changes both the result and module loading",
      }),
    ).toBeInViewport();
    const titleGeometry = await reader
      .locator("h2")
      .first()
      .evaluate((node) => ({
        width: node.getBoundingClientRect().width,
        available: node.parentElement!.getBoundingClientRect().width,
      }));
    expect(titleGeometry.width).toBeGreaterThan(titleGeometry.available / 2);
    const reading = await reader.boundingBox();
    console.log(
      "reading-height",
      JSON.stringify({
        width,
        height,
        actual: reading?.height,
        minimumReadingHeight,
      }),
    );
    expect(reading?.height).toBeGreaterThan(minimumReadingHeight);
    expect(
      reading
        ? Math.min(reading.y + reading.height, height) - Math.max(reading.y, 0)
        : 0,
    ).toBeGreaterThan(minimumReadingHeight);
    if (width <= 768) {
      const browse = page.getByRole("button", { name: "Browse topics" });
      await browse.click();
      const topic = page
        .getByRole("list", { name: "Tour steps" })
        .getByRole("button")
        .first();
      await expect(topic).toBeInViewport();
      expect(
        await topic
          .locator("span")
          .nth(1)
          .evaluate((node) => getComputedStyle(node).whiteSpace),
      ).toBe("normal");
      await topic.focus();
      await page.keyboard.press("Enter");
      await expect(browse).toBeFocused();
      await expect(browse).toBeVisible();
    }
    await page.locator('[data-block-id="visual"]').scrollIntoViewIfNeeded();
    const iframe = page.locator('iframe[title="Tour visual"]');
    const originalFrame = await iframe.elementHandle();
    const counter = page
      .frameLocator('iframe[title="Tour visual"]')
      .locator("#rich-counter");
    await counter.click();
    await expect(counter).toHaveText("1");
    await page.setViewportSize({ width: 390, height: 420 });
    await expect(canvasSwitch).toBeVisible();
    await canvasSwitch.focus();
    await page.keyboard.press("Space");
    await expect(canvas).toBeVisible();
    await reader.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    const scroll = await reader.evaluate((node) => node.scrollTop);
    await page.getByRole("radio", { name: "Chat", exact: true }).focus();
    await page.keyboard.press("Space");
    await expect(chat).toHaveValue("Unsent responsive draft");
    await page.getByRole("radio", { name: "Canvas", exact: true }).focus();
    await page.keyboard.press("Space");
    await expect(counter).toHaveText("1");
    expect(
      await originalChat?.evaluate(
        (node) =>
          node ===
          document.querySelector('textarea[aria-label="Chat composer"]'),
      ),
    ).toBe(true);
    expect(
      await originalFrame?.evaluate(
        (node) =>
          node === document.querySelector('iframe[title="Tour visual"]'),
      ),
    ).toBe(true);
    expect(
      await reader.evaluate(
        (node, before) => Math.abs(node.scrollTop - before),
        scroll,
      ),
    ).toBeLessThanOrEqual(2);
    const last = canvas
      .locator(":scope > header")
      .locator("button:not([disabled]), input:not([disabled])")
      .last();
    await last.focus();
    await expect(last).toBeInViewport();
    expect(
      await last.evaluate((node) => {
        const box = node.getBoundingClientRect();
        const header = node.closest("header")!.getBoundingClientRect();
        return box.left >= header.left - 1 && box.right <= header.right + 1;
      }),
    ).toBe(true);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}
