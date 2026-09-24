import { expect, test } from "@playwright/test";

const ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled&activity_fixture";

test.describe("Buddy Activity conversation history", () => {
  for (const width of [360, 1280]) {
    test(`shows accessible capped human-readable workflow history at ${width}px`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(ROUTE);
      await page.getByRole("tab", { name: "Activity", exact: true }).click();

      const panel = page.getByTestId("buddy-recent-chats");
      await expect(panel).toBeVisible();
      const list = panel.getByRole("list");
      await expect(list.getByRole("listitem")).toHaveCount(20);
      await expect(
        panel.getByRole("button", { name: "Show more" }),
      ).toBeVisible();

      const text = await panel.innerText();
      expect(text).toContain("1 entry");
      expect(text).toMatch(/2 chats/);
      expect(text).not.toMatch(/\b[a-z]+(?: [a-z]+){2,} \(/);
      expect(text).not.toMatch(/\b1 entries\b|\b1 messages\b/);
      expect(text).not.toContain("1 message");
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(width);

      await panel.getByRole("button", { name: "Show more" }).click();
      await expect(list.getByRole("listitem")).toHaveCount(40);
    });
  }
});
