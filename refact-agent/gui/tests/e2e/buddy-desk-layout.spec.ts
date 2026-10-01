import { expect, test, type Page } from "@playwright/test";

const BUDDY_ROUTE =
  "/tests/e2e/route-showcase.html?route=buddy&operator=enabled";
const DESK = "[aria-label='Buddy Desk']";
const CONTENT = "[data-testid='buddy-home-content']";
const FOLD_PX = 900;
const project = { project_id: "showcase", canonical_root: "/workspace/refact" };

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
    id: "garden-orphans",
    title: "Knowledge base needs attention: 97 lifecycle candidate(s)",
    state: "needs_attention",
    minutesAgo: 11,
    severity: "high",
    scope: "knowledge/index",
  },
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
    id: "workspace-warmup",
    title: "Warm up this workspace",
    state: "needs_attention",
    minutesAgo: 34,
    scope: "workspace setup",
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
    id: "parity-goal",
    title: "Finish multi-agent parity M1–M10",
    state: "needs_attention",
    minutesAgo: 92,
    scope: "multi-agent parity",
  },
  {
    id: "provider-key",
    title: "Waiting on a provider key",
    state: "blocked",
    minutesAgo: 140,
    severity: "low",
    scope: "provider credentials",
  },
  {
    id: "diagnostic-cluster",
    title: "Review the long-running diagnostic cluster that keeps reopening",
    state: "needs_attention",
    minutesAgo: 190,
    severity: "normal",
    scope: "diagnostics",
  },
  {
    id: "tidy",
    title: "Tidy",
    state: "needs_attention",
    minutesAgo: 240,
    scope: "project cleanup",
  },
  {
    id: "memo-extraction",
    title: "memo_extraction — run outcome",
    state: "watching",
    minutesAgo: 12,
    scope: "memo extraction",
  },
  {
    id: "error-detective",
    title: "Error Detective",
    state: "watching",
    minutesAgo: 300,
    scope: "error analysis",
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

function minutes(value: number): string {
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
    last_material_change_at: minutes(seed.minutesAgo),
    severity: seed.severity ?? null,
    problem_summary: seed.problemSummary ?? null,
    scope: seed.scope ?? null,
  });
}

function emptyArtifactRecord(seed: Seed) {
  return record({
    schema_version: 1,
    artifact_id: seed.id,
    revision: 1,
    project,
    source_run_ids: [],
    title: seed.title,
    conclusion: "Nothing conclusive was saved.",
    delta: "",
    body: "",
    uncertainty: null,
    proposed_action_ids: [],
    evidence: [],
    provenance: {
      source_run_ids: [],
      evidence_refs: [],
      contains_untrusted_source: false,
    },
  });
}

async function installOperatorRoutes(page: Page) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/buddy/work") {
      const states =
        VIEW_STATES[url.searchParams.get("view") ?? "needs_you"] ?? [];
      return route.fulfill({
        json: envelope(
          SEEDS.filter((seed) => states.includes(seed.state)).map(workRecord),
        ),
      });
    }
    const artifact = /^\/v1\/buddy\/artifacts\/([^/]+)/.exec(url.pathname);
    if (artifact) {
      const seed = SEEDS.find((candidate) => candidate.id === artifact[1]);
      if (!seed)
        return route.fulfill({ status: 404, json: { detail: "gone" } });
      return route.fulfill({ json: envelope(emptyArtifactRecord(seed)) });
    }
    if (url.pathname.startsWith("/v1/")) {
      return route.fulfill({ status: 503, json: { detail: "fixture only" } });
    }
    return route.continue();
  });
}

async function openBuddy(
  page: Page,
  width: number,
  theme: "dark" | "light" = "dark",
) {
  await installOperatorRoutes(page);
  await page.setViewportSize({ width, height: FOLD_PX });
  await page.goto(`${BUDDY_ROUTE}&theme=${theme}`);
  await page.getByTestId("buddy-home-content").waitFor();
  await expect(page.locator(`${DESK} ul li button`)).toHaveCount(8);
}

async function deskMetrics(page: Page) {
  return page.evaluate((desk: string) => {
    const root = document.querySelector<HTMLElement>(desk);
    const row = root?.querySelector<HTMLElement>("ul li button");
    const name = row?.querySelector<HTMLElement>("span");
    const chevron = row?.querySelector<HTMLElement>("svg");
    const items = root?.querySelector<HTMLElement>("ul");
    const heading = root?.querySelector<HTMLElement>("h2");
    const views = root?.querySelector<HTMLElement>("nav");
    if (!root || !row || !name || !chevron || !heading || !views || !items)
      throw new Error("Desk geometry is unavailable");
    const rowBox = row.getBoundingClientRect();
    const nameBox = name.getBoundingClientRect();
    const chevronBox = chevron.getBoundingClientRect();
    const leftEdges = [heading, views, row].map((element) =>
      Math.round(element.getBoundingClientRect().left),
    );
    return {
      fill: (chevronBox.right - nameBox.left) / rowBox.width,
      gap: chevronBox.left - nameBox.right,
      measure: Number.parseFloat(getComputedStyle(items).maxWidth),
      rowWidth: rowBox.width,
      leftEdges,
      scrollWidth: document.documentElement.scrollWidth,
      clientWidth: document.documentElement.clientWidth,
    };
  }, DESK);
}

test.describe("Buddy Desk layout", () => {
  for (const width of [768, 1280, 1600]) {
    test(`row measure and chevron remain legible at ${width}px`, async ({
      page,
    }) => {
      await openBuddy(page, width);
      const metrics = await deskMetrics(page);
      expect(
        metrics.fill,
        `${width}px row fill ${metrics.fill}`,
      ).toBeGreaterThanOrEqual(0.6);
      if (width >= 1280)
        expect(
          metrics.gap,
          `${width}px chevron gap ${metrics.gap}`,
        ).toBeLessThan(48);
      if (width >= 1280) {
        expect(metrics.measure, `${width}px 72ch row measure`).toBeGreaterThan(
          0,
        );
        expect(
          metrics.rowWidth,
          `${width}px triaged row width ${metrics.rowWidth}`,
        ).toBeLessThanOrEqual(metrics.measure);
      }
    });
  }

  for (const theme of ["dark", "light"] as const) {
    test(`Desk uses one content rail at 1280px in ${theme} theme`, async ({
      page,
    }) => {
      await openBuddy(page, 1280, theme);
      const desktop = await deskMetrics(page);
      const placeholder = page.locator(`${DESK} [class*='placeholder']`);
      await expect(placeholder).toBeVisible();
      const placeholderLeft = await placeholder.evaluate((element) =>
        Math.round(element.getBoundingClientRect().left),
      );
      const rails = [...new Set([...desktop.leftEdges, placeholderLeft])];
      expect(
        rails.length,
        `${theme} Desk left rails: ${rails.join(", ")}`,
      ).toBeLessThanOrEqual(2);
    });

    test(`Desk avoids narrow overflow at 360px in ${theme} theme`, async ({
      page,
    }) => {
      await openBuddy(page, 360, theme);
      const narrow = await deskMetrics(page);
      expect(
        narrow.fill,
        `${theme} 360px row fill ${narrow.fill}`,
      ).toBeGreaterThanOrEqual(0.6);
      expect(narrow.scrollWidth).toBeLessThanOrEqual(narrow.clientWidth + 1);
    });
  }

  test("switching views preserves the Desk's page geometry", async ({
    page,
  }) => {
    await openBuddy(page, 1280);
    const rows = page.locator(`${DESK} ul li button`);
    await rows.first().click();
    await expect(page.getByLabel("Work detail")).toBeVisible();
    await page.locator(CONTENT).evaluate((element) => element.scrollTo(0, 200));
    const selectionBefore = await page.locator(CONTENT).evaluate((element) => ({
      top: element.scrollTop,
      desk: Math.round(
        document
          .querySelector<HTMLElement>("[aria-label='Buddy Desk']")!
          .getBoundingClientRect().height,
      ),
    }));
    await page
      .getByRole("button", { name: "Needs you", exact: true })
      .evaluate((button) => (button as HTMLButtonElement).click());
    await expect(page.getByLabel("Work detail")).toHaveCount(0);
    const selectionAfter = await page.locator(CONTENT).evaluate((element) => ({
      top: element.scrollTop,
      desk: Math.round(
        document
          .querySelector<HTMLElement>("[aria-label='Buddy Desk']")!
          .getBoundingClientRect().height,
      ),
      activeRow:
        document.activeElement ===
        document.querySelector("[aria-label='Buddy Desk'] ul li button"),
    }));
    expect(Math.abs(selectionAfter.top - selectionBefore.top)).toBeLessThan(48);
    expect(Math.abs(selectionAfter.desk - selectionBefore.desk)).toBeLessThan(
      48,
    );
    expect(selectionAfter.activeRow).toBe(true);

    const before = await page.locator(CONTENT).evaluate((element) => ({
      top: element.scrollTop,
      height: element.scrollHeight,
      desk: Math.round(
        document
          .querySelector<HTMLElement>("[aria-label='Buddy Desk']")!
          .getBoundingClientRect().height,
      ),
    }));
    for (const view of ["Working on", "Delivered", "Watching", "Needs you"]) {
      await page
        .getByRole("button", { name: view, exact: true })
        .evaluate((button) => (button as HTMLButtonElement).click());
      await page.waitForTimeout(50);
      const after = await page.locator(CONTENT).evaluate((element) => ({
        top: element.scrollTop,
        height: element.scrollHeight,
        desk: Math.round(
          document
            .querySelector<HTMLElement>("[aria-label='Buddy Desk']")!
            .getBoundingClientRect().height,
        ),
      }));
      expect(
        Math.abs(after.top - before.top),
        `${view} scroll delta ${before.top} -> ${after.top}`,
      ).toBeLessThan(48);
      expect(
        Math.abs(after.desk - before.desk),
        `${view} Desk height delta ${before.desk} -> ${after.desk}`,
      ).toBeLessThan(48);
      expect(
        Math.abs(after.height - before.height),
        `${view} content height delta ${before.height} -> ${after.height}`,
      ).toBeLessThan(48);
    }
  });
});
