import { expect, test } from "@playwright/test";

const ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled&activity_fixture";
const REAL_CONVERSATIONS_ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled&conversations_fixture=real";

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

    test(`renders real-shaped workflow rows consistently at ${width}px`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(REAL_CONVERSATIONS_ROUTE);
      await page.getByRole("tab", { name: "Activity", exact: true }).click();

      const panel = page.getByTestId("buddy-recent-chats");
      const rows = panel.getByRole("listitem");
      await expect(rows).toHaveCount(5);
      const expectedNames = [
        { index: 0, name: "Memo" },
        { index: 1, name: "Knowledge" },
        { index: 2, name: "Commit Msg" },
        { index: 3, name: "Privacy observation degraded" },
        { index: 4, name: "Title generation" },
      ];
      const rowButtons = panel.getByRole("button", { name: /^Open / });
      await expect(rowButtons).toHaveCount(4);

      for (const { index, name } of expectedNames) {
        const rowText = await rows.nth(index).innerText();
        expect(rowText.toLowerCase().split(name.toLowerCase()).length - 1).toBe(
          1,
        );
      }
      await expect(rows.nth(0)).toContainText("Failed");
      await expect(rows.nth(1)).toContainText("Failed");
      await expect(rows.nth(2)).toContainText("Failed");
      await expect(rows.nth(3)).toContainText("1 message");
      await expect(rows.nth(3)).not.toContainText("Failed");
      await expect(rows.nth(4)).not.toContainText("Failed");
      await expect(
        rows.nth(3).getByRole("button", { name: /^Open / }),
      ).toHaveCount(0);
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(width);
    });
  }
});
