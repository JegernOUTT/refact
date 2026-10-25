import { test, expect } from "@playwright/test";

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Goal receipts are bounded and keyboard accessible at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-goal&theme=${theme}`,
      );
      await page
        .locator('[data-density="compact"] button[aria-expanded]')
        .first()
        .click();
      await page
        .getByRole("button", { name: "More details", exact: true })
        .click();
      const receipt = page.getByLabel("Goal receipt", { exact: true });
      await expect(receipt).toBeVisible();
      await expect(receipt).toContainText("Paused");
      await expect(receipt).toContainText("3210");
      const body = receipt.getByLabel("Goal content", { exact: true });
      expect((await body.textContent())?.length).toBe(4096);
      const next = receipt.getByRole("button", { name: "Next section" });
      await next.focus();
      await expect(next).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(body).toContainText("Second goal section");
      await expect(body.locator("script")).toHaveCount(0);
      await expect(next).toBeDisabled();
      await expect(receipt).toContainText("cannot grant permissions");
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}
