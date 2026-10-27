import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const rows = Array.from({ length: 8 }, (_, index) => ({
  value: {
    schema_version: 1,
    id: `idle-work-${index}`,
    project,
    subject: {
      resource_kind: "saved_report",
      resource_id: `idle-work-${index}`,
      version: "1",
    },
    kind: "assessment",
    title: `Idle report ${index}`,
    state: "needs_attention",
    native_sources: [],
    current_artifact_id: `idle-artifact-${index}`,
    current_artifact_revision: 1,
    last_material_change_at: new Date().toISOString(),
  },
  revision: 1,
  change_seq: 1,
}));

type RequestRecord = { path: string; search: string; phase: string };

function envelope(data: unknown) {
  return { schema_version: 1, project, data, next_cursor: null };
}

async function openDesk(
  page: Page,
  requests: RequestRecord[],
  getPhase: () => string,
) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.startsWith("/v1/buddy/"))
      requests.push({
        path: url.pathname,
        search: url.search,
        phase: getPhase(),
      });
    if (url.pathname === "/v1/buddy/work") {
      await route.fulfill({ json: envelope(rows) });
      return;
    }
    if (url.pathname === "/v1/buddy/work/actions") {
      const ids = url.searchParams.get("ids")?.split(",") ?? [];
      await route.fulfill({
        json: envelope(
          ids.map((work_id) => ({ work_id, actions: [], error: null })),
        ),
      });
      return;
    }
    if (/^\/v1\/buddy\/work\/[^/]+\/actions$/.test(url.pathname))
      return route.fulfill({ json: envelope([]) });
    if (url.pathname === "/v1/buddy/decisions/current")
      return route.fulfill({
        json: {
          schema_version: 1,
          project_root: project.canonical_root,
          coverage: "loaded_native_sessions",
          truncated: false,
          questions: [],
        },
      });
    if (url.pathname === "/v1/buddy/native-work")
      return route.fulfill({
        json: {
          schema_version: 1,
          project_root: project.canonical_root,
          coverage: "loaded_native_sessions",
          next_cursor: null,
          sessions: [],
        },
      });
    if (url.pathname === "/v1/buddy/approvals/current")
      return route.fulfill({
        json: {
          schema_version: 1,
          project_root: project.canonical_root,
          engine_identity: "showcase-engine",
          coverage: "loaded_native_sessions",
          truncated: false,
          warnings: [],
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
  await page.bringToFront();
  await page.getByRole("tab", { name: "Reports", exact: true }).click();
  await expect(
    page.locator('[aria-label="Buddy Desk"] ul li button'),
  ).toHaveCount(rows.length);
}

test.describe("Buddy idle request behavior", () => {
  test("loads row actions once and pauses native-work polling while hidden", async ({
    page,
  }) => {
    await page.clock.install();
    let phase = "setup";
    const requests: RequestRecord[] = [];
    await openDesk(page, requests, () => phase);
    const batchActionRequests = () =>
      requests.filter(({ path }) => path === "/v1/buddy/work/actions");
    const perRowActionRequests = () =>
      requests.filter(({ path }) =>
        /^\/v1\/buddy\/work\/[^/]+\/actions$/.test(path),
      );
    const nativeWorkRequests = (name: string) =>
      requests.filter(
        ({ path, phase: requestPhase }) =>
          path === "/v1/buddy/native-work" && requestPhase === name,
      );
    expect(batchActionRequests()).toHaveLength(1);
    expect(perRowActionRequests()).toHaveLength(0);
    const batchUrl = new URLSearchParams(batchActionRequests()[0].search);
    expect([...batchUrl.keys()].sort()).toEqual(["ids", "project_id"]);
    expect(batchUrl.get("project_id")).toBe("showcase");
    expect(batchUrl.get("ids")).toBe(
      rows.map(({ value }) => value.id).join(","),
    );
    expect(nativeWorkRequests("setup").length).toBeGreaterThan(0);

    phase = "visible";
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        get: () => "visible",
      });
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => false,
      });
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("visibilitychange"));
    });
    await page.getByRole("tab", { name: "Home", exact: true }).click();
    const summary = page.getByText("Live chats, goals and workers");
    await summary.click();
    const detail = page.locator("details").filter({ has: summary });
    await expect(detail).toHaveAttribute("open", "");
    await expect(
      page.getByText("No loaded sessions on this page."),
    ).toBeVisible();
    let visibleCount = nativeWorkRequests("visible").length;
    for (let interval = 0; interval < 2; interval += 1) {
      await page.clock.runFor(5000);
      await expect
        .poll(() => nativeWorkRequests("visible").length)
        .toBeGreaterThan(visibleCount);
      visibleCount = nativeWorkRequests("visible").length;
    }
    expect(visibleCount).toBeGreaterThanOrEqual(2);
    expect(batchActionRequests()).toHaveLength(1);
    expect(perRowActionRequests()).toHaveLength(0);

    phase = "hidden";
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        get: () => "hidden",
      });
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => true,
      });
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("visibilitychange"));
    });
    await page.waitForTimeout(250);
    const hiddenBaseline = requests.filter(
      ({ path }) => path === "/v1/buddy/native-work",
    ).length;
    for (let interval = 0; interval < 3; interval += 1) {
      await page.clock.runFor(5000);
      await page.waitForTimeout(250);
      expect(
        requests.filter(({ path }) => path === "/v1/buddy/native-work"),
      ).toHaveLength(hiddenBaseline);
    }
    expect(nativeWorkRequests("hidden")).toHaveLength(0);
    expect(batchActionRequests()).toHaveLength(1);
    expect(perRowActionRequests()).toHaveLength(0);

    phase = "resumed";
    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", {
        configurable: true,
        get: () => "visible",
      });
      Object.defineProperty(document, "hidden", {
        configurable: true,
        get: () => false,
      });
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("visibilitychange"));
    });
    await expect
      .poll(() => nativeWorkRequests("resumed").length)
      .toBeGreaterThanOrEqual(1);
    expect(batchActionRequests()).toHaveLength(1);
    expect(perRowActionRequests()).toHaveLength(0);
  });
});
