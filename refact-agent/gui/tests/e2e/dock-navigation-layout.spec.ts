import { expect, test, type Page, type Route } from "@playwright/test";

const parentId = "showcase-chat";
const deniedId = "dock-layout-denied";
const openedId = "dock-layout-opened";
const cors = { "Access-Control-Allow-Origin": "*" };

const agent = (childId: string, title: string) => ({
  agent_id: `agent-${childId}`,
  parent_chat_id: parentId,
  child_chat_id: childId,
  kind: "subagent",
  status: "running",
  title,
  progress: null,
  step_count: 0,
  last_activity: null,
  target_files: [],
  edited_files: [],
  diff_summary: null,
  conflict_summary: null,
  result_summary: null,
  error: null,
  started_at: null,
  finished_at: null,
  change_seq: 1,
});

const viewports = [
  { name: "desktop", width: 1280, height: 800 },
  { name: "sheet", width: 360, height: 740 },
];

async function rect(locator: ReturnType<Page["locator"]>) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  return box ?? { x: 0, y: 0, width: 0, height: 0 };
}

async function expectStackedAbove(
  page: Page,
  notice: ReturnType<Page["locator"]>,
) {
  const section = page.getByTestId("workspace-dock-section");
  const scope = page.getByRole("group", { name: "Agent work scope" });
  const sectionBox = await rect(section);
  const noticeBox = await rect(notice);
  const scopeBox = await rect(scope);
  expect(Math.abs(noticeBox.x - scopeBox.x)).toBeLessThanOrEqual(1);
  expect(
    Math.abs(noticeBox.x + noticeBox.width - (scopeBox.x + scopeBox.width)),
  ).toBeLessThanOrEqual(1);
  expect(noticeBox.y + noticeBox.height).toBeLessThanOrEqual(scopeBox.y + 0.5);
  expect(noticeBox.width).toBeGreaterThan(sectionBox.width * 0.8);
  expect(noticeBox.height).toBeLessThan(sectionBox.height / 2);
  return scopeBox;
}

for (const viewport of viewports) {
  test(`dock navigation notices stack above agent scope (${viewport.name})`, async ({
    page,
  }) => {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    });
    let releaseDenied: () => void = () => undefined;
    const deniedHeld = new Promise<void>((resolve) => {
      releaseDenied = resolve;
    });
    await page.route("**/v1/background-agents?**", (route: Route) =>
      route.fulfill({
        headers: cors,
        json: [
          agent(deniedId, "Denied child"),
          agent(openedId, "Opened child"),
        ],
      }),
    );
    await page.route(
      `**/v1/trajectories/${deniedId}?metadata_only=true`,
      async (route) => {
        await deniedHeld;
        await route.fulfill({
          status: 403,
          headers: cors,
          json: { detail: "Access denied" },
        });
      },
    );
    await page.route(
      `**/v1/trajectories/${openedId}?metadata_only=true`,
      (route) =>
        route.fulfill({
          headers: cors,
          json: {
            id: openedId,
            title: "Opened child",
            model: "",
            mode: "AGENT",
          },
        }),
    );

    await page.goto("/tests/e2e/route-showcase.html?route=chat");
    const closeSheet = page.getByRole("button", {
      name: "Close workspace panel",
    });
    if (await closeSheet.isVisible()) {
      await closeSheet.click();
      await expect(closeSheet).toHaveCount(0);
    }
    await page.getByRole("button", { name: "Agents", exact: true }).click();
    const scope = page.getByRole("group", { name: "Agent work scope" });
    await expect(scope).toBeVisible();
    await page.waitForFunction(() =>
      document
        .getAnimations()
        .every(
          (item) =>
            item.playState !== "running" ||
            item.effect?.getComputedTiming().endTime === Infinity,
        ),
    );
    const initialScope = await rect(scope);

    await page
      .getByRole("button", { name: "Denied child", exact: true })
      .click();
    const status = page
      .getByTestId("workspace-dock-section")
      .getByRole("status")
      .filter({ hasText: "Opening linked target..." });
    await expect(status).toBeVisible();
    const pendingScope = await expectStackedAbove(page, status);
    expect(
      Math.abs(pendingScope.width - initialScope.width),
    ).toBeLessThanOrEqual(1);

    releaseDenied();
    const alert = page.getByTestId("workspace-dock-section").getByRole("alert");
    await expect(alert).toContainText("Access denied");
    await expect(status).toHaveCount(0);
    const errorScope = await expectStackedAbove(page, alert);
    expect(Math.abs(errorScope.width - initialScope.width)).toBeLessThanOrEqual(
      1,
    );
    await expect(
      page.getByRole("button", { name: "Denied child", exact: true }),
    ).toBeVisible();
    await expect(
      page.locator(`[data-surface-key='chat:${deniedId}']`),
    ).toHaveCount(0);

    await page
      .getByRole("button", { name: "Opened child", exact: true })
      .click();
    await expect(status).toHaveCount(0);
    await expect(alert).toHaveCount(0);
    await expect(
      page.locator(`[data-surface-key='chat:${openedId}']`).first(),
    ).toBeAttached();
  });
}
