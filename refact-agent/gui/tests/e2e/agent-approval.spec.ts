import { test, expect } from "@playwright/test";

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Approval receipts stay bounded and keyboard accessible at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-approval&theme=${theme}`,
      );
      const card = page.getByLabel("Child command approval");
      await expect(card).toBeVisible();
      await expect(card).toContainText("first-request");
      await expect(card).not.toContainText("second-request");
      const next = card.getByRole("button", { name: "Next request" });
      await next.focus();
      await page.keyboard.press("Enter");
      await expect(card).toContainText("second-request");
      await expect(card).toContainText("does not prove command success");
      await expect(next).toBeDisabled();
      await card.getByRole("button", { name: "Previous request" }).click();
      await expect(card).toContainText("first-request");
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}
