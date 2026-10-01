import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = '[aria-label="Buddy Desk"]';
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const rows = Array.from({ length: 6 }, (_, index) => {
  const id = `narrow-work-${index}`;
  return {
    value: {
      schema_version: 1,
      id,
      project,
      subject: { resource_kind: "saved_report", resource_id: id, version: "1" },
      kind: "assessment",
      title: `Review saved finding ${index + 1}`,
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

async function openDesk(page: Page, width: number, height: number) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work")
      return route.fulfill({
        json: { schema_version: 1, project, data: rows, next_cursor: null },
      });
    return route.continue();
  });
  await page.setViewportSize({ width, height });
  const workList = page.waitForResponse(
    (response) => new URL(response.url()).pathname === "/v1/buddy/work",
  );
  await page.goto(ROUTE);
  await workList;
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(rows.length);
}

test.describe("Buddy Desk narrow layout", () => {
  test("compacts the controls and first work row into the first half-screen at 360px", async ({
    page,
  }) => {
    await openDesk(page, 360, 640);
    const row = page.locator(`${DESK} ul li button`).first();
    await expect(row).toBeVisible();
    const rowBox = await row.boundingBox();
    expect(rowBox, "first work row has no layout box").not.toBeNull();
    expect(rowBox!.y, `first work row starts at y=${rowBox!.y}`).toBeLessThan(
      320,
    );

    const viewButtons = page
      .getByRole("navigation", { name: "Work views" })
      .getByRole("button");
    await expect(viewButtons).toHaveText([
      "Needs you",
      "Working on",
      "Delivered",
      "Watching",
    ]);
    const viewBoxes = await viewButtons.evaluateAll((buttons) =>
      buttons.map((button) => {
        const { x, y, width } = button.getBoundingClientRect();
        return { x, y, width, right: x + width };
      }),
    );
    expect(
      Math.max(...viewBoxes.map(({ y }) => y)) -
        Math.min(...viewBoxes.map(({ y }) => y)),
    ).toBeLessThanOrEqual(2);
    for (const [index, box] of viewBoxes.entries()) {
      expect(
        box.x,
        `view button ${index} starts offscreen`,
      ).toBeGreaterThanOrEqual(0);
      expect(
        box.right,
        `view button ${index} ends offscreen`,
      ).toBeLessThanOrEqual(360);
    }

    const search = page.getByRole("searchbox", { name: "Search saved work" });
    const attention = page.getByLabel("Report attention");
    await expect(search).toBeVisible();
    await expect(attention).toBeVisible();
    for (const control of [search, attention]) {
      const box = await control.boundingBox();
      expect(box, "filter has no layout box").not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(360);
    }
    const widths = await page.evaluate(() => ({
      html: [
        document.documentElement.scrollWidth,
        document.documentElement.clientWidth,
      ],
      main: (() => {
        const element = document.querySelector<HTMLElement>("main");
        if (!element) throw new Error("main element is missing");
        return [element.scrollWidth, element.clientWidth];
      })(),
    }));
    expect(widths.html[0], "html has horizontal overflow").toBeLessThanOrEqual(
      widths.html[1],
    );
    expect(widths.main[0], "main has horizontal overflow").toBeLessThanOrEqual(
      widths.main[1],
    );
  });

  test("preserves the 1280px first-row position from base 581d8d090", async ({
    page,
  }) => {
    await openDesk(page, 1280, 900);
    const row = page.locator(`${DESK} ul li button`).first();
    const rowBox = await row.boundingBox();
    expect(rowBox, "first work row has no layout box").not.toBeNull();
    // Base commit 581d8d090 measured 226px for this fixture at 1280×900.
    expect(Math.abs(rowBox!.y - 226)).toBeLessThanOrEqual(2);
  });
});
