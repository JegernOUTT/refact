import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = '[aria-label="Buddy Desk"]';
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const titles = [
  "Diff growing fast",
  "Review saved provider configuration",
  "Investigate the build failure",
  "Check whether the source analysis still applies",
  "Compare the latest dependency recommendations",
  "Review repeated workspace initialization failures",
  "Investigate the long-running diagnostics and identify which subsystem should own recovery",
  "Determine why this unusually long-running diagnostic sequence keeps reopening across project sessions",
];
const rows = titles.map((title, index) => {
  const id = `realdata-work-${index}`;
  return {
    value: {
      schema_version: 1,
      id,
      project,
      subject: { resource_kind: "saved_report", resource_id: id, version: "1" },
      kind: "assessment",
      title,
      state: "needs_attention",
      native_sources: [],
      current_artifact_id: null,
      current_artifact_revision: null,
      last_material_change_at: new Date(
        Date.now() - index * 60_000,
      ).toISOString(),
      severity: null,
      problem_summary: null,
      scope: null,
    },
    revision: 1,
    change_seq: index + 1,
  };
});

async function openDesk(page: Page, width: number) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work")
      return route.fulfill({
        json: { schema_version: 1, project, data: rows, next_cursor: null },
      });
    return route.continue();
  });
  await page.setViewportSize({ width, height: 900 });
  const workList = page.waitForResponse(
    (response) => new URL(response.url()).pathname === "/v1/buddy/work",
  );
  await page.goto(ROUTE);
  await workList;
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(rows.length);
}

async function viewMetrics(page: Page) {
  return page.evaluate((deskSelector) => {
    const desk = document.querySelector<HTMLElement>(deskSelector);
    const nav = desk?.querySelector<HTMLElement>("nav");
    const list = desk?.querySelector<HTMLElement>("ul");
    if (!desk || !nav || !list)
      throw new Error("Buddy Desk view controls are missing");
    const navBox = nav.getBoundingClientRect();
    const buttons = [...nav.querySelectorAll<HTMLElement>("button")].map(
      (button) => {
        const box = button.getBoundingClientRect();
        const label = button.querySelector<HTMLElement>("span");
        if (!label) throw new Error("Work view button label is missing");
        return {
          left: box.left,
          right: box.right,
          top: box.top,
          labelWidth: label.clientWidth,
          labelScrollWidth: label.scrollWidth,
        };
      },
    );
    return {
      navLeft: navBox.left,
      navRight: navBox.right,
      navWidth: nav.clientWidth,
      navScrollWidth: nav.scrollWidth,
      buttons,
      listWidth: list.clientWidth,
      listScrollWidth: list.scrollWidth,
      deskWidth: desk.clientWidth,
      deskScrollWidth: desk.scrollWidth,
      mainWidth: document.querySelector<HTMLElement>("main")?.clientWidth ?? 0,
      mainScrollWidth:
        document.querySelector<HTMLElement>("main")?.scrollWidth ?? 0,
      documentWidth: document.documentElement.clientWidth,
      documentScrollWidth: document.documentElement.scrollWidth,
    };
  }, DESK);
}

async function chevronMetrics(page: Page) {
  return page.evaluate((deskSelector) => {
    const desk = document.querySelector<HTMLElement>(deskSelector);
    const list = desk?.querySelector<HTMLElement>("ul");
    if (!desk || !list) throw new Error("Buddy Desk work list is missing");
    const listWidth = list.getBoundingClientRect().width;
    const rows = [...list.querySelectorAll<HTMLElement>("li button")].map(
      (row) => {
        const chevron = row.querySelector<SVGElement>("svg");
        if (!chevron) throw new Error("Work row chevron is missing");
        return {
          rowWidth: row.getBoundingClientRect().width,
          chevronX: chevron.getBoundingClientRect().x,
        };
      },
    );
    return { listWidth, rows };
  }, DESK);
}

for (const width of [240, 280, 320, 360]) {
  test(`keeps all view buttons visible without horizontal overflow at ${width}px`, async ({
    page,
  }) => {
    await openDesk(page, width);
    const buttons = page
      .getByRole("navigation", { name: "Work views" })
      .getByRole("button");
    await expect(buttons).toHaveText([
      "Needs you",
      "Working on",
      "Delivered",
      "Watching",
    ]);
    const metrics = await viewMetrics(page);
    expect(
      metrics.navScrollWidth,
      `${width}px view nav overflow`,
    ).toBeLessThanOrEqual(metrics.navWidth);
    for (const [index, button] of metrics.buttons.entries()) {
      expect(
        button.left,
        `${width}px view ${index} left edge`,
      ).toBeGreaterThanOrEqual(metrics.navLeft - 1);
      expect(
        button.right,
        `${width}px view ${index} right edge`,
      ).toBeLessThanOrEqual(metrics.navRight + 1);
      expect(
        button.labelScrollWidth,
        `${width}px view ${index} label clipping`,
      ).toBeLessThanOrEqual(button.labelWidth);
    }
    expect(
      metrics.listScrollWidth,
      `${width}px list overflow`,
    ).toBeLessThanOrEqual(metrics.listWidth);
    expect(
      metrics.deskScrollWidth,
      `${width}px Desk overflow`,
    ).toBeLessThanOrEqual(metrics.deskWidth);
    expect(
      metrics.mainScrollWidth,
      `${width}px main overflow`,
    ).toBeLessThanOrEqual(metrics.mainWidth);
    expect(
      metrics.documentScrollWidth,
      `${width}px document overflow`,
    ).toBeLessThanOrEqual(metrics.documentWidth);
    if (width === 360)
      expect(
        Math.max(...metrics.buttons.map(({ top }) => top)) -
          Math.min(...metrics.buttons.map(({ top }) => top)),
      ).toBeLessThanOrEqual(2);
  });
}

for (const width of [360, 768, 1280]) {
  test(`anchors all work-row chevrons to the right edge at ${width}px`, async ({
    page,
  }) => {
    await openDesk(page, width);
    const metrics = await chevronMetrics(page);
    const positions = metrics.rows.map(({ chevronX }) => chevronX);
    expect(
      Math.max(...positions) - Math.min(...positions),
      `${width}px chevron spread ${positions.join(", ")}`,
    ).toBeLessThanOrEqual(1);
    for (const [index, row] of metrics.rows.entries()) {
      expect(
        Math.abs(row.rowWidth - metrics.listWidth),
        `${width}px row ${index} width ${row.rowWidth} vs list ${metrics.listWidth}`,
      ).toBeLessThanOrEqual(1);
    }
  });
}
