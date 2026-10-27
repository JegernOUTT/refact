// Characterizes the five-section Buddy Home with the operator Desk enabled and
// a large (50 item) saved-work backlog: default Home stays a bounded preview,
// the unbounded Desk only appears under Reports, and Play mounts the full
// scene at the top of the first viewport.
import { expect, test, type Page } from "@playwright/test";

const ROUTE = "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = '[aria-label="Buddy Desk"]';
const SECTIONS = ["Home", "Reports", "Activity", "Play", "Settings"] as const;
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

const workRow = (index: number, state: "needs_attention" | "delivered") => {
  const id = `redesign-${state}-${index}`;
  return {
    value: {
      schema_version: 1,
      id,
      project,
      subject: { resource_kind: "saved_report", resource_id: id, version: "1" },
      kind: "assessment",
      title: `${
        state === "delivered" ? "Delivered" : "Needs you"
      } report ${index} with a deliberately long title that must wrap at narrow widths`,
      state,
      native_sources: [],
      current_artifact_id: null,
      current_artifact_revision: null,
      last_material_change_at: new Date(
        Date.now() - index * 60_000,
      ).toISOString(),
      severity: null,
      problem_summary: "A long summary line that exercises wrapping in rows.",
      scope: "refact-agent/gui",
    },
    revision: 1,
    change_seq: index + 1,
  };
};

const NEEDS_YOU = Array.from({ length: 50 }, (_, i) =>
  workRow(i, "needs_attention"),
);
const DELIVERED = Array.from({ length: 50 }, (_, i) => workRow(i, "delivered"));

type WorkRequest = {
  view: string | null;
  limit: number | null;
  attention: string | null;
};

async function openBuddy(page: Page, width: number, height = 900) {
  const workRequests: WorkRequest[] = [];
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.route("**/v1/buddy/work?**", async (route) => {
    const url = new URL(route.request().url());
    const limitParam = url.searchParams.get("limit");
    workRequests.push({
      view: url.searchParams.get("view"),
      limit: limitParam === null ? null : Number(limitParam),
      attention: url.searchParams.get("attention"),
    });
    // Deliberately ignore `limit` and return the whole backlog so the UI's
    // own bound is exercised, not only the request argument.
    const data =
      url.searchParams.get("view") === "delivered" ? DELIVERED : NEEDS_YOU;
    return route.fulfill({
      json: { schema_version: 1, project, data, next_cursor: 50 },
    });
  });
  await page.setViewportSize({ width, height });
  await page.goto(ROUTE);
  await page.getByTestId("buddy-home-content").waitFor();
  return workRequests;
}

const outsideViewport = (page: Page, selectors: string[]) =>
  page.evaluate((entries) => {
    const width = document.documentElement.clientWidth;
    const issues: string[] = [];
    if (document.documentElement.scrollWidth > width + 1)
      issues.push(`document ${document.documentElement.scrollWidth}>${width}`);
    for (const selector of entries) {
      const el = document.querySelector(selector);
      if (!el) {
        issues.push(`${selector} missing`);
        continue;
      }
      const rect = el.getBoundingClientRect();
      if (rect.left < -1 || rect.right > width + 1)
        issues.push(
          `${selector} ${Math.round(rect.left)}..${Math.round(
            rect.right,
          )}/${width}`,
        );
      const sectionsStrip =
        el.getAttribute("role") === "tablist" &&
        el.getAttribute("aria-label") === "Buddy sections" &&
        ["auto", "scroll"].includes(getComputedStyle(el).overflowX);
      if (!sectionsStrip && el.scrollWidth > el.clientWidth + 2)
        issues.push(`${selector} inner ${el.scrollWidth}>${el.clientWidth}`);
    }
    return issues;
  }, selectors);

test.describe("Buddy Home redesign (operator enabled, 50 saved items)", () => {
  for (const width of [240, 360, 1280]) {
    test(`default Home is a bounded preview without the Desk at ${width}px`, async ({
      page,
    }) => {
      const workRequests = await openBuddy(page, width);

      const tabs = page
        .getByRole("tablist", { name: "Buddy sections" })
        .getByRole("tab");
      await expect(tabs).toHaveText([...SECTIONS]);
      await expect(
        page.getByRole("tab", { name: "Home", exact: true }),
      ).toHaveAttribute("aria-selected", "true");

      const saved = page.getByTestId("buddy-home-saved-work");
      await expect(saved).toBeVisible();
      const preview = saved.getByRole("list", { name: "Saved work preview" });
      await expect(preview.getByRole("listitem").first()).toBeVisible();
      expect(await preview.getByRole("listitem").count()).toBeLessThanOrEqual(
        3,
      );
      await expect(
        saved.getByRole("button", { name: "More reports" }),
      ).toBeVisible();

      // The unbounded Desk is not on Home.
      await expect(page.locator(DESK)).toHaveCount(0);
      // Home renders only bounded queries: unsnoozed needs-you capped at 3.
      const needsYou = workRequests.filter((r) => r.view === "needs_you");
      expect(needsYou.length).toBeGreaterThan(0);
      for (const request of needsYou) {
        expect(request.limit).toBe(3);
        expect(request.attention).toBe("unsnoozed");
      }
      for (const request of workRequests) {
        expect(request.limit, JSON.stringify(request)).not.toBeNull();
        expect(request.limit!).toBeLessThanOrEqual(10);
      }

      // A modest real scene on Home, exactly one world mounted.
      await expect(page.getByTestId("buddy-home-hero")).toBeVisible();
      await expect(page.getByTestId("buddy-world")).toHaveCount(1);

      expect(
        await outsideViewport(page, [
          "[data-testid='buddy-home-content']",
          "[role='tablist'][aria-label='Buddy sections']",
          "[data-testid='buddy-home-overview']",
          "[data-testid='buddy-home-saved-work']",
        ]),
      ).toEqual([]);

      for (const name of SECTIONS) {
        const tab = page.getByRole("tab", { name, exact: true });
        await tab.focus();
        await expect(tab).toBeFocused();
        const box = await tab.boundingBox();
        expect(box, name).not.toBeNull();
        expect(box!.x, name).toBeGreaterThanOrEqual(-1);
        expect(box!.x + box!.width, name).toBeLessThanOrEqual(width + 1);
      }
      expect(
        await page.evaluate(() => document.scrollingElement?.scrollLeft ?? 0),
      ).toBe(0);
    });
  }

  test("Reports owns the Desk; More reports navigates there", async ({
    page,
  }) => {
    await openBuddy(page, 360);
    await page
      .getByTestId("buddy-home-saved-work")
      .getByRole("button", { name: "More reports" })
      .click();
    await expect(
      page.getByRole("tab", { name: "Reports", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("buddy-home-reports")).toBeVisible();
    await expect(page.locator(DESK)).toBeVisible();
    await expect(page.getByTestId("buddy-world")).toHaveCount(0);
    // Legacy Pulse/Opportunities are not shown when the operator Desk owns
    // Reports.
    await expect(page.getByTestId("buddy-pulse-card")).toHaveCount(0);

    // Top-level navigation keeps working from inside a long Desk.
    await page.getByRole("tab", { name: "Settings", exact: true }).click();
    await expect(page.getByTestId("buddy-home-settings-section")).toBeVisible();
    await expect(page.getByTestId("buddy-settings-panel")).toBeVisible();
    await expect(page.locator(DESK)).toHaveCount(0);
    await page.getByRole("tab", { name: "Home", exact: true }).click();
    await expect(page.getByTestId("buddy-home-overview")).toBeVisible();
    await expect(page.locator(DESK)).toHaveCount(0);

    expect(
      await outsideViewport(page, [
        "[data-testid='buddy-home-content']",
        "[role='tablist'][aria-label='Buddy sections']",
      ]),
    ).toEqual([]);
  });

  test("Activity shows read-only warnings and recent chats with operator on", async ({
    page,
  }) => {
    await openBuddy(page, 360);
    await page.getByRole("tab", { name: "Activity", exact: true }).click();
    await expect(page.getByTestId("buddy-home-activity")).toBeVisible();
    await expect(page.getByTestId("buddy-recent-errors-panel")).toHaveCount(0);
    await expect(page.locator(DESK)).toHaveCount(0);
    expect(
      await outsideViewport(page, [
        "[data-testid='buddy-home-content']",
        "[data-testid='buddy-home-activity']",
      ]),
    ).toEqual([]);
  });

  for (const viewport of [
    { width: 360, height: 640 },
    { width: 1280, height: 800 },
  ]) {
    test(`Play mounts the full scene at the top of the first viewport at ${viewport.width}px`, async ({
      page,
    }) => {
      await openBuddy(page, viewport.width, viewport.height);
      await page.getByRole("tab", { name: "Play", exact: true }).click();
      const play = page.getByTestId("buddy-home-play");
      await expect(play).toBeVisible();
      const world = play.getByTestId("buddy-world");
      await expect(world).toHaveCount(1);
      await expect(page.getByTestId("buddy-world")).toHaveCount(1);
      await expect(world).toBeInViewport();
      const box = await world.boundingBox();
      expect(box).not.toBeNull();
      // Scene starts in the upper part of the first viewport and is not a
      // thumbnail.
      expect(box!.y).toBeLessThan(viewport.height / 2);
      expect(box!.height).toBeGreaterThan(120);
      // Scene comes before the play controls and summary strip.
      const strip = await play.getByTestId("buddy-summary-strip").boundingBox();
      if (strip) expect(strip.y).toBeGreaterThan(box!.y);
      await expect(page.locator(DESK)).toHaveCount(0);
      expect(
        await outsideViewport(page, [
          "[data-testid='buddy-home-content']",
          "[data-testid='buddy-home-play']",
        ]),
      ).toEqual([]);
    });
  }
});
