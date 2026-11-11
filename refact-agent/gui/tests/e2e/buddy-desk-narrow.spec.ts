import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = '[data-testid="buddy-desk"]';
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
  await page.route("**/v1/buddy/operator/pending-publications", (route) =>
    route.fulfill({ json: { schema_version: 1, project, data: [] } }),
  );
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work")
      return route.fulfill({
        json: { schema_version: 1, project, data: rows, next_cursor: null },
      });
    return route.fallback();
  });
  await page.setViewportSize({ width, height });
  const workList = page.waitForResponse(
    (response) => new URL(response.url()).pathname === "/v1/buddy/work",
  );
  const recovery = page.waitForResponse(
    "**/v1/buddy/operator/pending-publications",
  );
  await page.goto(ROUTE);
  await workList;
  expect((await recovery).ok()).toBe(true);
  await page.getByRole("tab", { name: "Reports", exact: true }).click();
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(rows.length);
  await expect(
    page.getByText(/Publication recovery is unavailable/),
  ).toHaveCount(0);
}

async function expectFirstRowBelowControlsInTopHalf(
  page: Page,
  height: number,
) {
  const row = page.locator(`${DESK} ul li button`).first();
  await expect(row).toBeVisible();
  await expect(row).toBeInViewport();
  const rowBox = await row.boundingBox();
  await test.info().attach("first-report-row", {
    body: JSON.stringify(rowBox),
    contentType: "application/json",
  });
  expect(rowBox, "first work row has no layout box").not.toBeNull();
  expect(rowBox!.y, `first work row starts at y=${rowBox!.y}`).toBeLessThan(
    height / 2,
  );
  const controlsBottom = await page.evaluate((desk) => {
    const root = document.querySelector<HTMLElement>(desk);
    const list = root?.querySelector("ul");
    if (!root || !list) throw new Error("Desk controls are missing");
    return Math.max(
      ...[
        ...root.querySelectorAll<HTMLElement>(
          '[role="radiogroup"], input:not([type="radio"]), select, button',
        ),
      ]
        .filter(
          (control) =>
            !list.contains(control) &&
            control.compareDocumentPosition(list) &
              Node.DOCUMENT_POSITION_FOLLOWING,
        )
        .map((control) => control.getBoundingClientRect().bottom),
    );
  }, DESK);
  expect(
    rowBox!.y,
    `first work row overlaps controls ending at y=${controlsBottom}`,
  ).toBeGreaterThanOrEqual(controlsBottom - 1);
}

test.describe("Reports narrow layout", () => {
  for (const width of [240, 1280]) {
    test(`keeps the first work row below usable controls at ${width}px`, async ({
      page,
    }) => {
      await openDesk(page, width, 900);
      await expectFirstRowBelowControlsInTopHalf(page, 900);
      for (const control of [
        page.getByRole("searchbox", { name: "Search reports" }),
        page.getByLabel("Show reports"),
      ]) {
        await expect(control).toBeVisible();
        const box = await control.boundingBox();
        expect(box).not.toBeNull();
        expect(box!.x).toBeGreaterThanOrEqual(0);
        expect(box!.x + box!.width).toBeLessThanOrEqual(width);
      }
    });
  }

  test("compacts the controls and first work row into the first half-screen at 360px", async ({
    page,
  }) => {
    await openDesk(page, 360, 900);
    await expect(page.getByTestId("buddy-desk-detail-hint")).toBeHidden();
    await expectFirstRowBelowControlsInTopHalf(page, 900);

    const viewButtons = page
      .getByRole("radiogroup", { name: "Report views" })
      .getByRole("radio");
    await expect(viewButtons).toHaveCount(4);
    for (const name of ["Needs you", "Working on", "Delivered", "Watching"])
      await expect(
        page.getByRole("radio", { name, exact: true }),
      ).toBeVisible();
    for (const name of ["Needs you", "Working on", "Delivered", "Watching"]) {
      const control = page.getByRole("radio", { name, exact: true });
      await control.locator("..").scrollIntoViewIfNeeded();
      await control.focus();
      const box = await control.locator("..").boundingBox();
      expect(box).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(360);
    }

    const search = page.getByRole("searchbox", { name: "Search reports" });
    const attention = page.getByLabel("Show reports");
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
});
