import { expect, test } from "@playwright/test";

for (const colorScheme of ["light", "dark"] as const) {
  for (const width of [240, 360, 768, 1280]) {
    test(`layout warning stays above chat at ${width}px in ${colorScheme}`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme });
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=chat&app_workspace_status=error&theme=${colorScheme}`,
      );
      const notice = page.getByRole("status", { name: "App layout" });
      const retry = notice.getByRole("button", { name: "Retry save" });
      const pane = page.getByRole("region", { name: "Workspace pane root" });
      await expect(notice).toBeVisible();
      await expect(notice).toContainText("Your local layout is preserved.");
      await expect(retry).toBeInViewport();
      await retry.focus();
      await expect(retry).toBeFocused();
      const noticeBox = await notice.boundingBox();
      const paneBox = await pane.boundingBox();
      const retryBox = await retry.boundingBox();
      const messageBox = await notice
        .getByText(
          "App layout could not be saved. Your local layout is preserved.",
        )
        .boundingBox();
      expect(noticeBox).not.toBeNull();
      expect(paneBox).not.toBeNull();
      expect(retryBox).not.toBeNull();
      expect(messageBox).not.toBeNull();
      if (!noticeBox || !paneBox || !retryBox || !messageBox) return;
      expect(noticeBox.x).toBeGreaterThanOrEqual(0);
      expect(noticeBox.x + noticeBox.width).toBeLessThanOrEqual(width);
      expect(messageBox.width).toBeGreaterThanOrEqual(
        Math.min(120, noticeBox.width / 2),
      );
      expect(paneBox.y).toBeGreaterThanOrEqual(noticeBox.y + noticeBox.height);
      expect(paneBox.height).toBeGreaterThan(300);
      expect(retryBox.y + retryBox.height).toBeLessThanOrEqual(
        noticeBox.y + noticeBox.height,
      );
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
    });
  }
}

test("read-only layout notice has no misleading retry action", async ({
  page,
}) => {
  await page.goto(
    "/tests/e2e/route-showcase.html?route=chat&app_workspace_status=readonly",
  );
  const notice = page.getByRole("status", { name: "App layout" });
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("read-only");
  await expect(notice.getByRole("button")).toHaveCount(0);
});
