import { test, expect } from "@playwright/test";

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Message receipts stay bounded and keyboard accessible at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-message&theme=${theme}`,
      );
      const receipt = page.getByLabel("Agent message receipt", { exact: true });
      await expect(receipt).toBeVisible();
      await expect(receipt).toContainText("Message queued");
      const body = receipt.getByLabel("Requested message content", {
        exact: true,
      });
      expect((await body.textContent())?.length).toBe(4096);
      const next = receipt.getByRole("button", { name: "Next section" });
      await next.focus();
      await expect(next).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(body).toContainText("Second message section");
      await expect(body.locator("script")).toHaveCount(0);
      await expect(next).toBeDisabled();
      await expect(receipt).toContainText("not proof of consumption");
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });

    test(`Board broadcast receipts page per card and stay inert at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-broadcast&theme=${theme}`,
      );
      const receipt = page.getByLabel("Agent message receipt", { exact: true });
      await expect(receipt).toBeVisible();
      await expect(receipt).toContainText("4 of 7 board agents reached");
      const cards = receipt.getByLabel("Board delivery receipts", {
        exact: true,
      });
      await expect(cards).toContainText("Card 0 <script>inert</script>");
      await expect(cards).toContainText("Not delivered: no live agent session");
      await expect(cards.locator("script")).toHaveCount(0);
      await expect(cards).not.toContainText("Card 5");
      const next = cards.getByRole("button", { name: "Next cards" });
      await next.focus();
      await expect(next).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(cards).toContainText("Card 5");
      await expect(next).toBeDisabled();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}
