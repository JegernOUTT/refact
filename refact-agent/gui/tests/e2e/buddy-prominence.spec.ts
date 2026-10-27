import { expect, test, type Page } from "@playwright/test";

/**
 * Visual prominence must track usefulness. Measured on a live page at
 * 1280x900 before this suite existed, the two largest regions were the two
 * least useful ones: `buddy-recent-chats` at 1192x468 held a single row, and
 * the scene occupied ~26% of the viewport while sitting between the work list
 * and the controls that act on it.
 *
 * Everything here MEASURES. Nothing infers prominence from class names.
 */

const BUDDY_ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = "[aria-label='Buddy Desk']";
const SAVED = "[data-testid='buddy-home-saved-work']";
const HOME_SCENE =
  "[data-testid='buddy-home-overview'] [data-testid='buddy-home-hero']";
const RECENT = "[data-testid='buddy-recent-chats']";

const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

/** One conversation inside a 468px panel was ~400px of reserved nothing. */
const ONE_ROW_PANEL_LIMIT_PX = 200;
const MAX_EMPTY_RATIO = 0.4;

type Seed = {
  id: string;
  title: string;
  state: "needs_attention" | "blocked" | "watching";
  minutesAgo: number;
  severity?: "low" | "normal" | "high" | "critical";
  problemSummary?: string;
  scope?: string;
};

const SEEDS: Seed[] = [
  {
    id: "garden-conflicts",
    title: "Two saved memories disagree about the release tag scheme",
    state: "needs_attention",
    minutesAgo: 11,
    severity: "critical",
    problemSummary: "Release metadata disagrees across saved sources.",
    scope: "release/config",
  },
  {
    id: "git-uncommitted",
    title: "5681 uncommitted files",
    state: "needs_attention",
    minutesAgo: 58,
    severity: "high",
    scope: "working tree",
  },
  {
    id: "provider-key",
    title: "Waiting on a provider key",
    state: "blocked",
    minutesAgo: 140,
    severity: "low",
    scope: "provider credentials",
  },
];

const VIEW_STATES: Record<string, Seed["state"][]> = {
  needs_you: ["needs_attention", "blocked"],
  working_on: [],
  delivered: [],
  watching: ["watching"],
};

function envelope(data: unknown) {
  return { schema_version: 1, project, data, next_cursor: null };
}

function record(value: unknown) {
  return { value, revision: 1, change_seq: 1 };
}

function minutesAgo(value: number): string {
  return new Date(Date.now() - value * 60_000).toISOString();
}

function workRecord(seed: Seed) {
  return record({
    schema_version: 1,
    id: seed.id,
    project,
    subject: {
      resource_kind: "saved_report",
      resource_id: seed.id,
      version: "1",
    },
    kind: "assessment",
    title: seed.title,
    state: seed.state,
    native_sources: [],
    current_artifact_id: null,
    current_artifact_revision: null,
    last_material_change_at: minutesAgo(seed.minutesAgo),
    severity: seed.severity ?? null,
    problem_summary: seed.problemSummary ?? null,
    scope: seed.scope ?? null,
  });
}

async function openBuddyWithWork(page: Page, width = 1280) {
  await page.route("**/v1/buddy/work*", (route) => {
    const url = new URL(route.request().url());
    const states =
      VIEW_STATES[url.searchParams.get("view") ?? "needs_you"] ?? [];
    return route.fulfill({
      json: envelope(
        SEEDS.filter((seed) => states.includes(seed.state)).map(workRecord),
      ),
    });
  });
  await page.setViewportSize({ width, height: 900 });
  await page.goto(BUDDY_ROUTE);
  // The machine can be under load; gate on readiness, never on a longer clock.
  await page.getByTestId("buddy-home-content").waitFor();
  await expect(
    page.getByTestId("buddy-home-saved-work").getByRole("listitem").first(),
  ).toBeVisible();
}

/** Rendered area in px², or null when the region is not on the page. */
function areaOf(page: Page, selector: string) {
  return page.evaluate((target: string) => {
    const element = document.querySelector<HTMLElement>(target);
    if (!element) return null;
    const box = element.getBoundingClientRect();
    return Math.round(box.width * box.height);
  }, selector);
}

test.describe("Buddy home prominence", () => {
  test.beforeEach(({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("saved_work_outranks_the_home_scene_by_area", async ({ page }) => {
    await openBuddyWithWork(page);

    const workArea = await areaOf(page, SAVED);
    const sceneArea = await areaOf(page, HOME_SCENE);

    expect(workArea, "Home must preview saved work").not.toBeNull();
    expect(
      sceneArea,
      "the compact scene must stay on Home; this is a prominence change, not a removal",
    ).not.toBeNull();
    expect(
      workArea,
      `saved work ${workArea}px² vs Home scene ${sceneArea}px²`,
    ).toBeGreaterThan(sceneArea ?? 0);
  });

  test("home_scene_is_compact_and_play_expands_it", async ({ page }) => {
    await openBuddyWithWork(page);

    await expect(page.getByTestId("buddy-world")).toHaveCount(1);
    await expect(page.getByTestId("buddy-world")).toBeVisible();
    const homeWorldArea = await areaOf(page, "[data-testid='buddy-world']");

    await page.getByRole("button", { name: "Open Play", exact: true }).click();
    await expect(
      page.getByRole("tab", { name: "Play", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("buddy-world")).toHaveCount(1);
    const playWorldArea = await areaOf(page, "[data-testid='buddy-world']");
    expect(
      playWorldArea,
      `Play produced ${playWorldArea}px² against a Home ${homeWorldArea}px²`,
    ).toBeGreaterThan(homeWorldArea ?? 0);
    await expect(
      page.getByRole("button", { name: "Close Play", exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Reset Buddy scene", exact: true }),
    ).toBeVisible();

    await page.getByRole("button", { name: "Leave Play", exact: true }).click();
    await expect(
      page.getByRole("tab", { name: "Home", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(page.getByTestId("buddy-world")).toHaveCount(1);
  });

  test("recent_chats_does_not_reserve_empty_space", async ({ page }) => {
    await openBuddyWithWork(page);
    await page.getByRole("tab", { name: "Activity", exact: true }).click();

    const panel = page.locator(RECENT);
    await expect(panel).toBeVisible();
    // The showcase serves its conversation fixture from its own fetch shim, so
    // the one-row case is reached through the panel's own filter rather than a
    // route override that would never fire.
    await panel
      .getByRole("radio", { name: "Chats", exact: true })
      .click({ force: true });
    await expect(panel.locator("[data-clickable]")).toHaveCount(1);

    const geometry = await panel.evaluate((element) => {
      const panelHeight = element.getBoundingClientRect().height;
      const used = [...element.children].reduce(
        (total, child) => total + child.getBoundingClientRect().height,
        0,
      );
      return {
        height: Math.round(panelHeight),
        used: Math.round(used),
      };
    });

    expect(
      geometry.height,
      `a one-row Recent chats panel is ${geometry.height}px tall; the measured original was 468px holding a single row`,
    ).toBeLessThan(ONE_ROW_PANEL_LIMIT_PX);
    const emptyRatio = 1 - geometry.used / geometry.height;
    expect(
      emptyRatio,
      `${Math.round(emptyRatio * 100)}% of the panel is empty: ${
        geometry.used
      }px of content inside ${geometry.height}px`,
    ).toBeLessThanOrEqual(MAX_EMPTY_RATIO);
  });

  test("reports_shows_the_desk_without_a_competing_scene", async ({ page }) => {
    await openBuddyWithWork(page);
    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    await expect(page.locator(`${DESK} ul li button`).first()).toBeVisible();
    await expect(page.getByTestId("buddy-world")).toHaveCount(0);
    const deskTop = await page
      .locator(DESK)
      .evaluate((element) => Math.round(element.getBoundingClientRect().top));
    expect(deskTop, `Desk top ${deskTop}`).toBeLessThan(450);
  });
});
