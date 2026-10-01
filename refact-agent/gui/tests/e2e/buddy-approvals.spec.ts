import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";

async function openApprovalsError(page: Page) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/approvals/current")
      return route.fulfill({ status: 503, json: { detail: "unavailable" } });
    if (url.pathname === "/v1/buddy/decisions/current")
      return route.fulfill({
        json: {
          schema_version: 1,
          project_root: "/workspace/refact",
          coverage: "loaded_native_sessions",
          truncated: false,
          questions: [],
        },
      });
    if (url.pathname === "/v1/buddy/native-work")
      return route.fulfill({
        json: {
          schema_version: 1,
          project_root: "/workspace/refact",
          coverage: "loaded_native_sessions",
          next_cursor: null,
          sessions: [],
        },
      });
    if (url.pathname === "/v1/buddy/supervision/context")
      return route.fulfill({
        json: {
          schema_version: 1,
          project: null,
          worker: null,
          authenticated_owner: null,
          features: {
            storage: false,
            actions: false,
            supervision: false,
            improvement: false,
            editorial: false,
          },
        },
      });
    if (url.pathname.startsWith("/v1/"))
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    return route.continue();
  });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(ROUTE);
  await expect(page.getByLabel("Native approvals")).toBeVisible();
}

test.describe("Buddy native approval Retry reachability", () => {
  for (const width of [360, 1280]) {
    test(`Retry is clickable at ${width}px`, async ({ page }) => {
      await openApprovalsError(page);
      await page.setViewportSize({ width, height: 900 });

      const retry = page
        .getByLabel("Native approvals")
        .getByRole("button", { name: "Retry", exact: true });
      await expect(retry).toBeVisible();
      await retry.click({ trial: true });

      const hitTarget = await retry.evaluate((button) => {
        const rect = button.getBoundingClientRect();
        const target = document.elementFromPoint(
          rect.left + rect.width / 2,
          rect.top + rect.height / 2,
        );
        return target === button || button.contains(target);
      });
      expect(hitTarget).toBe(true);
    });
  }
});
