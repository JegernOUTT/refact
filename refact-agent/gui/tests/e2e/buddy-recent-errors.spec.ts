import { expect, test } from "@playwright/test";

const ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled&recent_errors_fixture";

test.describe("Buddy Recent errors", () => {
  for (const width of [360, 1280]) {
    test(`shows an accessible, deduplicated list at ${width}px`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(ROUTE);
      await page.getByRole("tab", { name: "Activity", exact: true }).click();

      const list = page.getByRole("list", { name: "Recent warnings" });
      await expect(list.getByRole("listitem")).toHaveCount(20);
      await expect(
        list.getByText("Model unavailable", { exact: true }),
      ).toBeVisible();
      const titleSummaryRow = list
        .getByRole("listitem")
        .filter({ hasText: "Model unavailable" });
      await expect(
        titleSummaryRow.getByText("Model unavailable", { exact: true }),
      ).toHaveCount(1);
      await expect(
        titleSummaryRow.getByText("Model unavailable: Model unavailable"),
      ).toHaveCount(0);
      expect(await list.getByText("just now").count()).toBeGreaterThan(0);
      await expect(
        list.getByText("Provider unavailable ×3", { exact: true }),
      ).toBeVisible();
      await expect(
        list.getByText("Provider unavailable — retry later"),
      ).toHaveCount(0);
      await expect(
        list.getByText("Distinct warning 0: Additional detail 0"),
      ).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Show more" }),
      ).toBeVisible();
      expect(
        await page.evaluate(() => document.documentElement.scrollWidth),
      ).toBeLessThanOrEqual(width);

      await page.getByRole("button", { name: "Show more" }).click();
      await expect(list.getByRole("listitem")).toHaveCount(27);
    });
  }

  test("keeps all 27 warnings after Buddy plays through the runtime queue", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(ROUTE);
    await page.getByRole("tab", { name: "Activity", exact: true }).click();

    const list = page.getByRole("list", { name: "Recent warnings" });
    await expect(list.getByRole("listitem")).toHaveCount(20);
    await page.getByRole("button", { name: "Show more" }).click();
    await expect(list.getByRole("listitem")).toHaveCount(27);
    await page.evaluate(() => {
      const buddyStore = (
        window as unknown as {
          __routeShowcaseBuddyStore: {
            dispatch: (action: { type: string }) => void;
          };
        }
      ).__routeShowcaseBuddyStore;
      for (let index = 0; index < 27; index += 1) {
        buddyStore.dispatch({ type: "buddy/dequeueRuntimeEvent" });
        buddyStore.dispatch({ type: "buddy/clearNowPlaying" });
      }
    });

    await expect(list.getByRole("listitem")).toHaveCount(27);
  });
});
