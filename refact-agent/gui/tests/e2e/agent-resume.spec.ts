import { test, expect } from "@playwright/test";

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Resume receipt distinguishes cap and execution at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-resume&theme=${theme}`,
      );
      const card = page.getByLabel("Child resume", { exact: true });
      await expect(card).toBeVisible();
      await expect(card).toContainText("Inspection only");
      await expect(card.getByLabel("Execution cap")).toContainText("20 / 20");
      await expect(card).toContainText(
        "Unknown tool outcomes are never replayed",
      );
      const open = card.getByRole("button", { name: "Open child trajectory" });
      await open.focus();
      await expect(open).toBeFocused();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}
