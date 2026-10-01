import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = '[aria-label="Buddy Desk"]';
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

const titles = [
  "A short title",
  "Review saved provider configuration",
  "Investigate a persistent build failure",
  "Check whether the latest source analysis still applies",
  "A longer saved finding about repeated workspace initialization failures",
  "Review the dependency change that affects the entire application build pipeline",
  "Compare the saved recommendations against current project configuration and runtime evidence",
  "Investigate the unusually long-running diagnostic sequence and determine which subsystem should own its recovery behavior",
];

const rows = titles.map((title, index) => {
  const id = `row-work-${index}`;
  const artifactId = `row-artifact-${index}`;
  return {
    id,
    artifactId,
    title,
    label: `Review finding ${index + 1}`,
    value: {
      schema_version: 1,
      id,
      project,
      subject: {
        resource_kind: "saved_report",
        resource_id: id,
        version: "1",
      },
      kind: "assessment",
      title,
      state: "needs_attention",
      native_sources: [],
      current_artifact_id: index === 0 ? null : artifactId,
      current_artifact_revision: index === 0 ? null : 1,
      last_material_change_at: new Date(
        Date.now() - index * 60_000,
      ).toISOString(),
      severity: null,
      problem_summary: null,
      scope: `project area ${index + 1}`,
    },
  };
});

function envelope(data: unknown) {
  return { schema_version: 1, project, data, next_cursor: null };
}

function workRecord(row: (typeof rows)[number]) {
  return { value: row.value, revision: 1, change_seq: 1 };
}

async function openDesk(page: Page, width: number) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work")
      return route.fulfill({ json: envelope(rows.map(workRecord)) });
    if (url.pathname === "/v1/buddy/work/actions") {
      const ids = url.searchParams.get("ids")?.split(",") ?? [];
      return route.fulfill({
        json: envelope(
          ids.map((work_id) => ({
            work_id,
            actions: [],
            error: "fixture only",
          })),
        ),
      });
    }
    const actions = /^\/v1\/buddy\/work\/([^/]+)\/actions$/.exec(url.pathname);
    if (actions)
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    if (url.pathname.startsWith("/v1/"))
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    return route.continue();
  });
  await page.setViewportSize({ width, height: 900 });
  await page.goto(ROUTE);
  await page.getByTestId("buddy-home-content").waitFor();
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(rows.length);
  await expect(page.getByText("No result yet", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Action status unavailable", { exact: true }).first(),
  ).toBeVisible();
}

test.describe("Buddy Desk rows", () => {
  test("rows fill the list and keep action hints in metadata styling without overflow", async ({
    page,
  }) => {
    await openDesk(page, 240);
    for (const width of [240, 360, 768, 1280, 1600]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(page.locator(`${DESK} ul li button`)).toHaveCount(
        rows.length,
      );
      const measurements = await page.evaluate((deskSelector) => {
        const desk = document.querySelector<HTMLElement>(deskSelector);
        const list = desk?.querySelector<HTMLElement>("ul");
        if (!desk || !list) throw new Error("Buddy Desk rows are missing");
        const listWidth = list.getBoundingClientRect().width;
        const rowWidths = [...list.querySelectorAll("li button")].map(
          (row) => row.getBoundingClientRect().width,
        );
        const hint = desk.querySelector<HTMLElement>(
          "li button [class*='actionVerb']",
        );
        const relativeTime =
          desk.querySelector<HTMLElement>("li button [title]");
        if (!hint || !relativeTime)
          throw new Error("Row metadata styling targets are missing");
        return {
          listWidth,
          rowWidths,
          hintColor: getComputedStyle(hint).color,
          hintDecoration: getComputedStyle(hint).textDecorationLine,
          relativeColor: getComputedStyle(relativeTime).color,
          documentWidth: document.documentElement.scrollWidth,
          viewportWidth: document.documentElement.clientWidth,
        };
      }, DESK);
      expect(
        measurements.rowWidths.every(
          (rowWidth) => Math.abs(rowWidth - measurements.listWidth) <= 1,
        ),
        `${width}px row widths ${measurements.rowWidths.join(
          ", ",
        )} did not match list width ${measurements.listWidth}`,
      ).toBe(true);
      expect(measurements.hintColor, `${width}px action hint color`).toBe(
        measurements.relativeColor,
      );
      expect(measurements.hintDecoration).not.toBe("underline");
      expect(
        measurements.documentWidth,
        `${width}px document overflow: ${measurements.documentWidth} > ${measurements.viewportWidth}`,
      ).toBeLessThanOrEqual(measurements.viewportWidth + 1);
    }
  });

  test("Tab focus gives a row a visible two-pixel focus ring", async ({
    page,
  }) => {
    await openDesk(page, 1280);
    const firstRow = page.locator(`${DESK} ul li button`).first();
    let focused = false;
    for (let index = 0; index < 60; index += 1) {
      await page.keyboard.press("Tab");
      focused = await firstRow.evaluate(
        (row) => row === document.activeElement,
      );
      if (focused) break;
    }
    expect(focused, "Tab traversal did not reach the first Desk row").toBe(
      true,
    );
    const outlineWidth = await firstRow.evaluate((row) =>
      Number.parseFloat(getComputedStyle(row).outlineWidth),
    );
    expect(outlineWidth).toBeGreaterThanOrEqual(2);
  });
});
