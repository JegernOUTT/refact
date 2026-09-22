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
const SCENE = "[data-testid='buddy-home-scene']";
const RECENT = "[data-testid='buddy-recent-chats']";

const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

/** The collapsed strip merges the old 46px presence row; ~56px plus padding. */
const STRIP_BUDGET_PX = 96;
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
  await expect(page.locator(`${DESK} ul li button`).first()).toBeVisible();
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

  test("desk_outranks_the_scene_by_area", async ({ page }) => {
    await openBuddyWithWork(page);

    const deskArea = await areaOf(page, DESK);
    const sceneArea = await areaOf(page, SCENE);

    expect(deskArea, "the Desk must render when there is work").not.toBeNull();
    expect(
      sceneArea,
      "the scene must stay on the page; this is a prominence change, not a removal",
    ).not.toBeNull();
    expect(
      deskArea,
      `Desk ${deskArea}px² vs scene ${sceneArea}px²; the scene used to be rank 3 at ~328,160px² while the Desk was rank 2`,
    ).toBeGreaterThan(sceneArea ?? 0);
  });

  test("scene_is_collapsed_by_default_and_expandable", async ({ page }) => {
    await openBuddyWithWork(page);

    const scene = page.locator(SCENE);
    await expect(scene).toHaveAttribute("data-scene-view", "strip");

    const stripHeight = await scene.evaluate((element) =>
      Math.round(element.getBoundingClientRect().height),
    );
    expect(
      stripHeight,
      `collapsed scene strip is ${stripHeight}px tall; it replaces a 46px presence row plus a full scene card`,
    ).toBeLessThanOrEqual(STRIP_BUDGET_PX);

    // Collapsed still means mounted: the character keeps rendering.
    await expect(page.getByTestId("buddy-world")).toBeVisible();

    const collapsedWorldArea = await areaOf(
      page,
      "[data-testid='buddy-world']",
    );

    await page
      .getByRole("button", { name: "Expand scene", exact: true })
      .click();
    await expect(scene).toHaveAttribute("data-scene-view", "scene");
    const expandedWorldArea = await areaOf(page, "[data-testid='buddy-world']");
    expect(
      expandedWorldArea,
      `expanding produced ${expandedWorldArea}px² against a collapsed ${collapsedWorldArea}px²`,
    ).toBeGreaterThan(collapsedWorldArea ?? 0);

    await page
      .getByRole("button", { name: "Collapse scene", exact: true })
      .click();
    await expect(scene).toHaveAttribute("data-scene-view", "strip");

    // The merged mount contract is untouched: same labels, same semantics.
    await page.getByRole("button", { name: "Close Play", exact: true }).click();
    await expect(page.getByTestId("buddy-world")).toHaveCount(0);
    await expect(page.getByTestId("buddy-presence-avatar")).toBeVisible();
    await page.getByRole("button", { name: "Open Play", exact: true }).click();
    await expect(page.getByTestId("buddy-world")).toBeVisible();
    await expect(page.getByTestId("buddy-presence-avatar")).toHaveCount(0);

    // The Play tab still reaches the full scene.
    await page.getByRole("tab", { name: "Play", exact: true }).click();
    await expect(scene).toHaveAttribute("data-scene-view", "scene");
    await expect(page.getByTestId("buddy-world")).toBeVisible();
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

  test("work_precedes_the_scene_in_dom_order", async ({ page }) => {
    await openBuddyWithWork(page);

    const order = await page.evaluate(
      ([desk, scene]: string[]) => {
        const deskNode = document.querySelector(desk);
        const sceneNode = document.querySelector(scene);
        const tablist = document.querySelector("[role='tablist']");
        if (!deskNode || !sceneNode || !tablist) return null;
        return {
          deskBeforeScene: Boolean(
            deskNode.compareDocumentPosition(sceneNode) &
              Node.DOCUMENT_POSITION_FOLLOWING,
          ),
          // The merged layout contract: the scene still leads the tab strip.
          sceneBeforeTablist: Boolean(
            sceneNode.compareDocumentPosition(tablist) &
              Node.DOCUMENT_POSITION_FOLLOWING,
          ),
          deskTop: Math.round(deskNode.getBoundingClientRect().top),
          sceneTop: Math.round(sceneNode.getBoundingClientRect().top),
        };
      },
      [DESK, SCENE],
    );

    expect(order).not.toBeNull();
    expect(
      order?.deskBeforeScene,
      "the Desk must precede the scene; reading order used to be work rows -> snack request -> seasonal poetry -> Grant a bounded mandate",
    ).toBe(true);
    expect(order?.sceneBeforeTablist).toBe(true);
    expect(
      order?.sceneTop,
      `scene top ${order?.sceneTop} against Desk top ${order?.deskTop}`,
    ).toBeGreaterThan(order?.deskTop ?? 0);
  });
});
