import { test, expect } from "@playwright/test";

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Plan receipts remain bounded and keyboard accessible at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-plan&theme=${theme}`,
      );
      const receipt = page.getByLabel("Plan receipt", { exact: true });
      await expect(receipt).toBeVisible();
      await expect(receipt).toContainText("Read only");
      const body = receipt.getByLabel("Plan content", { exact: true });
      expect((await body.textContent())?.length).toBe(4096);
      const next = receipt.getByRole("button", { name: "Next section" });
      await next.focus();
      await expect(next).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(body).toContainText("Second plan section");
      await expect(next).toBeDisabled();
      await expect(receipt).toContainText("does not establish approval");
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}
