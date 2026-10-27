import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = '[aria-label="Buddy Desk"]';
const CONTENT = '[data-testid="buddy-home-content"]';
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };
const rows = Array.from({ length: 8 }, (_, index) => ({
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
    state: index === 7 ? "watching" : "needs_attention",
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

type Diagnostic = Record<string, string | number>;
type DiagnosticWindow = Window & { __buddyFocusLog: Diagnostic[] };

type FocusTarget = {
  tag: string;
  role: string;
  testId: string;
  label: string;
  text: string;
};

async function openBuddy(page: Page) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work") {
      const view = url.searchParams.get("view") ?? "needs_you";
      if (url.searchParams.get("search"))
        await new Promise((resolve) => setTimeout(resolve, 500));
      return route.fulfill({
        json: {
          schema_version: 1,
          project,
          data:
            view === "needs_you"
              ? rows.slice(0, 7)
              : view === "watching"
                ? rows.slice(7)
                : [],
          next_cursor: null,
        },
      });
    }
    if (url.pathname.startsWith("/v1/")) {
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    }
    return route.continue();
  });
  await page.addInitScript(() => {
    const log: Diagnostic[] = [];
    Object.assign(window, { __buddyFocusLog: log });
    const record = (type: string, target?: Element | null) => {
      log.push({
        type,
        time: performance.now(),
        tag: target?.tagName ?? "",
        role: target?.getAttribute("role") ?? "",
        testId: target?.getAttribute("data-testid") ?? "",
        label: target?.getAttribute("aria-label") ?? "",
        text: (target?.textContent ?? "")
          .trim()
          .replace(/\s+/g, " ")
          .slice(0, 60),
      });
    };
    document.addEventListener(
      "focusin",
      (event) =>
        record(
          "focusin",
          event.target instanceof Element ? event.target : null,
        ),
      true,
    );
    document.addEventListener(
      "focusout",
      (event) =>
        record(
          "focusout",
          event.target instanceof Element ? event.target : null,
        ),
      true,
    );
    document.addEventListener("selectionchange", () => {
      const selection = document.getSelection();
      log.push({
        type: "selectionchange",
        time: performance.now(),
        rangeCount: selection?.rangeCount ?? 0,
        anchorOffset: selection?.anchorOffset ?? 0,
        anchor: selection?.anchorNode?.textContent?.trim().slice(0, 60) ?? "",
      });
    });
    window.addEventListener("hashchange", () =>
      log.push({
        type: "hashchange",
        time: performance.now(),
        hash: location.hash,
      }),
    );
    const originalFocus = HTMLElement.prototype.focus;
    HTMLElement.prototype.focus = function (...args) {
      record("focus-call", this);
      return originalFocus.apply(this, args);
    };
    const originalScroll = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (...args) {
      record("scrollIntoView", this);
      return originalScroll.apply(this, args);
    };
    const originalReplace = History.prototype.replaceState;
    History.prototype.replaceState = function (...args) {
      log.push({
        type: "replaceState",
        time: performance.now(),
        url: String(args[2] ?? ""),
      });
      return originalReplace.apply(this, args);
    };
    const originalPush = History.prototype.pushState;
    History.prototype.pushState = function (...args) {
      log.push({
        type: "pushState",
        time: performance.now(),
        url: String(args[2] ?? ""),
      });
      return originalPush.apply(this, args);
    };
  });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(ROUTE);
  await expect(page.locator(CONTENT)).toBeVisible();
  await page.getByRole("tab", { name: "Reports", exact: true }).click();
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(7);
}

async function activeElement(
  page: Page,
): Promise<FocusTarget & { body: boolean }> {
  return page.evaluate(() => {
    const active = document.activeElement;
    return {
      body: active === document.body,
      tag: active?.tagName ?? "none",
      role: active?.getAttribute("role") ?? "",
      testId: active?.getAttribute("data-testid") ?? "",
      label: active?.getAttribute("aria-label") ?? "",
      text: (active?.textContent ?? "")
        .trim()
        .replace(/\s+/g, " ")
        .slice(0, 60),
    };
  });
}

async function countTabsToFirstRow(page: Page, limit: number) {
  const targets: FocusTarget[] = [];
  for (let count = 1; count <= limit; count += 1) {
    await page.keyboard.press("Tab");
    const target = await activeElement(page);
    targets.push(target);
    if (
      await page
        .locator(`${DESK} ul li button`)
        .first()
        .evaluate((row) => row === document.activeElement)
    )
      return { count, targets };
  }
  throw new Error(`First work row was not reached in ${limit} Tab stops`);
}

async function openSelectedItem(page: Page) {
  await page.locator(`${DESK} ul li button`).first().click();
  await expect(page.getByLabel("Work detail")).toBeVisible();
}

test.describe("Buddy keyboard access", () => {
  test("first_work_row_is_reachable_quickly", async ({ page }) => {
    await openBuddy(page);
    const freshLoad = await page.evaluate((deskSelector) => {
      const desk = document.querySelector<HTMLElement>(deskSelector);
      if (!desk) throw new Error("Buddy Desk is missing");
      const selection = document.getSelection();
      const style = getComputedStyle(desk);
      return {
        hash: location.hash,
        selectionRangeCount: selection?.rangeCount ?? 0,
        desk: {
          hidden: desk.hidden,
          inert: desk.inert,
          display: style.display,
          visibility: style.visibility,
        },
      };
    }, DESK);
    expect(freshLoad.hash).toBe("");
    expect(freshLoad.selectionRangeCount).toBe(0);
    expect(freshLoad.desk.hidden).toBe(false);
    expect(freshLoad.desk.inert).toBe(false);
    const rawLoad = await countTabsToFirstRow(page, 40);
    const rawFocusLog = await page.evaluate(
      () => (window as DiagnosticWindow).__buddyFocusLog,
    );
    console.log(
      "Raw fresh-load traversal",
      JSON.stringify({
        stops: rawLoad.count,
        targets: rawLoad.targets,
        focusLog: rawFocusLog,
      }),
    );
    expect(rawLoad.count).toBeLessThanOrEqual(12);
    expect(rawLoad.targets.slice(0, -1).map((target) => target.text)).toEqual(
      expect.arrayContaining([
        "Refresh work",
        "Needs you",
        "Working on",
        "Delivered",
        "Watching",
      ]),
    );
    expect(rawLoad.targets.some((target) => target.body)).toBe(false);

    await page.goto(ROUTE);
    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    await expect(page.locator(`${DESK} ul li button`)).toHaveCount(7);
    await page.evaluate(() => {
      const sentinel = document.createElement("button");
      sentinel.type = "button";
      sentinel.textContent = "Keyboard measurement start";
      sentinel.style.position = "fixed";
      sentinel.style.inset = "0 auto auto 0";
      document.body.insertBefore(sentinel, document.body.firstChild);
      sentinel.focus();
    });
    const trueStart = await countTabsToFirstRow(page, 20);
    console.log(
      "True document-start traversal",
      JSON.stringify({ stops: trueStart.count, targets: trueStart.targets }),
    );
    const reportsTab = trueStart.targets.findIndex(
      (target) => target.role === "tab" && target.text === "Reports",
    );
    expect(reportsTab, "Buddy sections tab strip is a single Tab stop").toBe(
      trueStart.targets.findIndex((target) => target.role === "tab"),
    );
    expect(trueStart.count - reportsTab - 1).toBe(rawLoad.count);
  });

  test("buddy_section_tabs_support_arrow_key_navigation", async ({ page }) => {
    await openBuddy(page);
    const reports = page.getByRole("tab", { name: "Reports", exact: true });
    await reports.focus();
    for (const name of ["Activity", "Play", "Settings", "Home", "Reports"]) {
      await page.keyboard.press("ArrowRight");
      const tab = page.getByRole("tab", { name, exact: true });
      await expect(tab).toBeFocused();
      await expect(tab).toHaveAttribute("aria-selected", "true");
    }
    await page.keyboard.press("ArrowLeft");
    await expect(
      page.getByRole("tab", { name: "Home", exact: true }),
    ).toBeFocused();
  });

  test("keeps_the_activated_view_focused_when_no_selection_is_open", async ({
    page,
  }) => {
    await openBuddy(page);
    for (const view of ["Watching", "Needs you", "Working on", "Delivered"]) {
      const control = page.getByRole("button", { name: view, exact: true });
      await control.click();
      await expect(control).toBeFocused();
      expect((await activeElement(page)).body).toBe(false);
    }
    for (const view of ["Watching", "Needs you", "Working on", "Delivered"]) {
      const control = page.getByRole("button", { name: view, exact: true });
      await control.focus();
      await page.keyboard.press("Enter");
      await expect(control).toBeFocused();
      expect((await activeElement(page)).body).toBe(false);
    }
  });

  test("switching_to_a_nonempty_view_with_a_selection_focuses_its_first_row", async ({
    page,
  }) => {
    await openBuddy(page);
    await openSelectedItem(page);
    await page.getByRole("button", { name: "Watching", exact: true }).click();
    const firstRow = page.locator(`${DESK} ul li button`).first();
    await expect(firstRow).toHaveCount(1);
    await expect(firstRow).toBeFocused();
    expect((await activeElement(page)).body).toBe(false);
  });

  test("switching_to_an_empty_view_with_a_selection_open_keeps_focus_on_the_view_control", async ({
    page,
  }) => {
    const workingOn = page.getByRole("button", {
      name: "Working on",
      exact: true,
    });
    await openBuddy(page);
    await openSelectedItem(page);
    await workingOn.click();
    await expect(page.locator(`${DESK} ul li button`)).toHaveCount(0);
    await expect(workingOn).toBeFocused();
    expect((await activeElement(page)).body).toBe(false);

    await page.getByRole("button", { name: "Needs you", exact: true }).click();
    await openSelectedItem(page);
    await workingOn.focus();
    await page.keyboard.press("Enter");
    await expect(page.locator(`${DESK} ul li button`)).toHaveCount(0);
    await expect(workingOn).toBeFocused();
    expect((await activeElement(page)).body).toBe(false);
  });

  test("typing_in_desk_search_after_closing_a_row_keeps_focus_in_the_search_box", async ({
    page,
  }) => {
    await openBuddy(page);
    await openSelectedItem(page);
    await page.getByRole("button", { name: "Back to work list" }).click();
    const search = page.locator(`${DESK} input[type="search"]`);
    await search.focus();
    const searchedResponse = page.waitForResponse((response) => {
      const url = new URL(response.url());
      return (
        url.pathname === "/v1/buddy/work" &&
        url.searchParams.get("search") === "needle"
      );
    });
    await search.fill("needle");
    const loading = page.getByText("Loading work…", { exact: true });
    await expect(loading).toBeVisible();
    await expect(search).toBeFocused();
    await searchedResponse;
    await expect(loading).toHaveCount(0);
    await expect(search).toBeFocused();
  });

  test("closing_a_selection_returns_focus_without_scrolling", async ({
    page,
  }) => {
    await openBuddy(page);
    const content = page.locator(CONTENT);
    await page.locator(`${DESK} ul li button`).first().focus();
    await page.keyboard.press("Enter");
    await expect(page.getByLabel("Work detail")).toBeVisible();
    const back = page.getByRole("button", { name: "Back to work list" });
    await expect(back).toBeFocused();
    await content.evaluate((element) => element.scrollTo(0, 200));
    const before = await content.evaluate((element) => element.scrollTop);
    await page.keyboard.press("Enter");
    await expect(page.getByLabel("Work detail")).toHaveCount(0);
    await expect(page.locator(`${DESK} ul li button`).first()).toBeFocused();
    const after = await content.evaluate((element) => element.scrollTop);
    expect(Math.abs(after - before)).toBeLessThan(48);
  });
});
