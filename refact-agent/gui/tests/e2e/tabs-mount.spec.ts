import { expect, test } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const rows = Array.from({ length: 7 }, (_, index) => ({
  value: {
    schema_version: 1,
    id: `keyboard-work-${index}`,
    project,
    subject: {
      resource_kind: "saved_report",
      resource_id: `keyboard-work-${index}`,
      version: "1",
    },
    kind: "assessment",
    title: `Keyboard work item ${index}`,
    state: "needs_attention",
    native_sources: [],
    current_artifact_id: null,
    current_artifact_revision: null,
    last_material_change_at: new Date().toISOString(),
    severity: null,
    problem_summary: null,
    scope: null,
  },
  revision: 1,
  change_seq: 1,
}));

test("shared Tabs does not scroll Buddy or the document on mount", async ({
  page,
}) => {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work")
      return route.fulfill({
        json: { schema_version: 1, project, data: rows, next_cursor: null },
      });
    if (url.pathname.startsWith("/v1/"))
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    return route.continue();
  });

  for (const [width, height] of [
    [360, 640],
    [768, 900],
    [1280, 900],
  ]) {
    await page.setViewportSize({ width, height });
    await page.goto(ROUTE);
    const desk = page.locator('[aria-label="Buddy Desk"]');
    await expect(desk).toBeVisible();
    await expect(desk.locator("ul li button")).toHaveCount(7);

    const scroll = await page.evaluate(() => ({
      content: document.querySelector<HTMLElement>(
        '[data-testid="buddy-home-content"]',
      )?.scrollTop,
      document: document.scrollingElement?.scrollTop,
    }));

    expect(scroll, `scroll position at ${width}×${height}`).toEqual({
      content: 0,
      document: 0,
    });
  }
});
