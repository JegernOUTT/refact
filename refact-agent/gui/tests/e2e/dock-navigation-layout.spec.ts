import { expect, test, type Route } from "@playwright/test";

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

for (const viewport of viewports) {
  test(`dock linked navigation preserves the source during preflight, denies and recovers (${viewport.name})`, async ({
    page,
  }) => {
    await page.setViewportSize({
      width: viewport.width,
      height: viewport.height,
    });
    let deniedRequested = false;
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
        deniedRequested = true;
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
    await page
      .getByRole("button", { name: "Denied child", exact: true })
      .click();
    const status = page
      .getByTestId("workspace-dock-section")
      .getByRole("status")
      .filter({ hasText: /Opening linked target/ });
    await expect.poll(() => deniedRequested).toBe(true);
    // Known roster children use local preflight without a global loading notice.
    await expect(status).toHaveCount(0);
    await expect(scope).toBeVisible();

    releaseDenied();
    const alert = page.getByTestId("workspace-dock-section").getByRole("alert");
    await expect(alert).toContainText("Access denied");
    await expect(status).toHaveCount(0);
    await expect(scope).toBeVisible();
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
