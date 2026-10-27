import { expect, test, type Page } from "@playwright/test";

const BUDDY_ROUTE = "/tests/e2e/route-showcase.html?route=buddy";
const WIDTHS = [360, 768, 1280, 1600] as const;
const THEMES = ["dark", "light"] as const;

const BROKEN_TABLIST_TOP_PX = 1695;
const FOLD_LIMIT_PX = 500;
const BROKEN_MAIN_SCROLL_WIDTH_PX = 466;
const BROKEN_MAIN_CLIENT_WIDTH_PX = 328;
const BROKEN_TRIGGER_WIDTH_PX = 38;
const MIN_TRIGGER_WIDTH_PX = 44;
const MIN_VISIBLE_CONTENT_PX = 200;

const OPPORTUNITY_TITLE =
  "Investigate a long-running diagnostic cluster before it becomes gremlin soup";

const REASSURING_EMPTY_COPY = [
  "No pending questions were found",
  "No approvals or warnings found",
];

const NATIVE_PANEL_LABELS = [
  "Native questions",
  "Native approvals",
  "Buddy supervision",
];

const EMPTY_DECISIONS = {
  schema_version: 1,
  project_root: "/workspace/refact",
  coverage: "loaded_native_sessions",
  truncated: false,
  questions: [],
};

const EMPTY_APPROVALS = {
  schema_version: 1,
  project_root: "/workspace/refact",
  engine_identity: "showcase-engine",
  coverage: "loaded_native_sessions",
  truncated: false,
  warnings: [],
  sessions: [],
};

const supervisionContext = (supervision: boolean) => ({
  schema_version: 1,
  project: null,
  worker: null,
  authenticated_owner: null,
  features: {
    storage: false,
    actions: false,
    supervision,
    improvement: false,
    editorial: false,
  },
});

type NativeFixture = "empty" | "unavailable" | "supervision-blocked";

async function installNativeRoutes(page: Page, fixture: NativeFixture) {
  const unavailable = fixture === "unavailable";
  await page.route("**/v1/buddy/decisions/current*", (route) =>
    unavailable
      ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
      : route.fulfill({ json: EMPTY_DECISIONS }),
  );
  await page.route("**/v1/buddy/approvals/current*", (route) =>
    unavailable
      ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
      : route.fulfill({ json: EMPTY_APPROVALS }),
  );
  await page.route("**/v1/buddy/supervision/context*", (route) =>
    unavailable
      ? route.fulfill({ status: 503, json: { detail: "unavailable" } })
      : route.fulfill({
          json: supervisionContext(fixture === "supervision-blocked"),
        }),
  );
}

async function openBuddy(
  page: Page,
  options: {
    width: number;
    theme?: (typeof THEMES)[number];
    fixture?: NativeFixture;
  },
) {
  await installNativeRoutes(page, options.fixture ?? "empty");
  await page.setViewportSize({ width: options.width, height: 900 });
  await page.goto(
    options.theme ? `${BUDDY_ROUTE}&theme=${options.theme}` : BUDDY_ROUTE,
  );
  await page.getByTestId("buddy-home-content").waitFor();
}

function readTabReach(page: Page) {
  return page.evaluate(() => {
    const tablist = document.querySelector<HTMLElement>("[role='tablist']");
    if (!tablist) return null;
    const style = window.getComputedStyle(tablist);
    return {
      scrollWidth: tablist.scrollWidth,
      clientWidth: tablist.clientWidth,
      scrollable: ["auto", "scroll"].includes(style.overflowX),
      unreachable: [...tablist.querySelectorAll<HTMLElement>("[role='tab']")]
        .filter(
          (trigger) =>
            trigger.offsetLeft + trigger.offsetWidth > tablist.scrollWidth + 1,
        )
        .map((trigger) => (trigger.textContent ?? "").trim()),
    };
  });
}

function readTabTriggers(page: Page) {
  return page.evaluate(() =>
    [...document.querySelectorAll("[role='tab']")].map((trigger) => {
      const label = trigger.firstElementChild ?? trigger;
      return {
        text: (trigger.textContent ?? "").trim(),
        width: Math.round(trigger.getBoundingClientRect().width),
        labelScrollWidth: label.scrollWidth,
        labelClientWidth: label.clientWidth,
      };
    }),
  );
}

test.describe("Buddy layout", () => {
  test.beforeEach(({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  for (const theme of THEMES) {
    for (const width of WIDTHS) {
      test(`keeps the scene above the tab strip and the scroller unbroken at ${width}px in ${theme}`, async ({
        page,
      }) => {
        await openBuddy(page, { width, theme });

        await expect(page.locator(`.radix-themes.${theme}`)).toHaveCount(1);
        await expect(page.getByTestId("buddy-world")).toBeVisible();
        await expect(page.getByTestId("buddy-home-hero")).toBeVisible();

        await expect(page.getByTestId("buddy-world")).toHaveCount(1);
        await expect(
          page.getByTestId("buddy-home-overview").getByTestId("buddy-world"),
        ).toBeVisible();
        await expect(
          page.getByRole("tab", { selected: true }),
        ).toHaveAccessibleName("Home");

        const scrollers = await page.evaluate(() => {
          const main = document.querySelector(
            "main[data-testid='buddy-home-content']",
          );
          const root = document.documentElement;
          if (!main) return null;
          return {
            mainScrollWidth: main.scrollWidth,
            mainClientWidth: main.clientWidth,
            rootScrollWidth: root.scrollWidth,
            rootClientWidth: root.clientWidth,
          };
        });
        expect(scrollers).not.toBeNull();
        expect(
          scrollers!.mainScrollWidth,
          `main[data-testid=buddy-home-content] overflowed: ${
            scrollers!.mainScrollWidth
          }x${
            scrollers!.mainClientWidth
          } (the broken build measured ${BROKEN_MAIN_SCROLL_WIDTH_PX}x${BROKEN_MAIN_CLIENT_WIDTH_PX} at 360px while documentElement looked clean)`,
        ).toBeLessThanOrEqual(scrollers!.mainClientWidth);
        expect(
          scrollers!.rootScrollWidth,
          `documentElement overflowed: ${scrollers!.rootScrollWidth}x${
            scrollers!.rootClientWidth
          }`,
        ).toBeLessThanOrEqual(scrollers!.rootClientWidth + 1);

        const triggers = await readTabTriggers(page);
        expect(triggers.length).toBeGreaterThanOrEqual(5);
        const labels = triggers.map((trigger) => trigger.text);
        expect(
          new Set(labels).size,
          `duplicate tab labels ${labels.join(
            " | ",
          )} — the broken build squeezed 8 triggers to ${BROKEN_TRIGGER_WIDTH_PX}px so "Personality" and "Pulse" both rendered "P…"`,
        ).toBe(labels.length);
        for (const trigger of triggers) {
          expect(
            trigger.text.length,
            `single-character tab label "${trigger.text}"`,
          ).toBeGreaterThan(1);
          expect(
            trigger.text,
            `ellipsised tab label "${trigger.text}"`,
          ).not.toMatch(/[…]|\.\.\.$/);
          expect(
            trigger.labelScrollWidth,
            `tab label "${trigger.text}" is clipped at ${trigger.labelScrollWidth}x${trigger.labelClientWidth}`,
          ).toBeLessThanOrEqual(trigger.labelClientWidth + 1);
          expect(
            trigger.width,
            `tab trigger "${trigger.text}" is ${trigger.width}px wide; the broken build rendered ${BROKEN_TRIGGER_WIDTH_PX}px`,
          ).toBeGreaterThanOrEqual(MIN_TRIGGER_WIDTH_PX);
        }

        const reach = await readTabReach(page);
        expect(reach).not.toBeNull();
        expect(
          reach!.unreachable,
          `tab triggers ${reach!.unreachable.join(
            ", ",
          )} sit outside the scrollable extent of their own tab strip, so no scrolling reaches them`,
        ).toEqual([]);
        expect(
          reach!.scrollWidth <= reach!.clientWidth + 1 || reach!.scrollable,
          `tab strip overflows (${reach!.scrollWidth} > ${
            reach!.clientWidth
          }) without being scrollable; distinct labels are not the same thing as reachable tabs`,
        ).toBe(true);

        for (const trigger of triggers) {
          const tab = page.getByRole("tab", {
            name: trigger.text,
            exact: true,
          });
          await tab.click();
          await expect(tab).toHaveAttribute("aria-selected", "true");
        }
      });
    }
  }

  test(`puts the default tab's content above the fold at 1280x900 (the tab strip alone was ${BROKEN_TABLIST_TOP_PX}px)`, async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280 });

    const geometry = await page.evaluate(() => {
      const tablist = document.querySelector("[role='tablist']");
      const panel = document.querySelector(
        "[role='tabpanel']:not([hidden])",
      ) as HTMLElement | null;
      if (!tablist || !panel) return null;
      const strip = tablist.getBoundingClientRect();
      const content = panel.getBoundingClientRect();
      return {
        stripTop: Math.round(strip.top),
        contentTop: Math.round(content.top),
        visibleContent: Math.round(
          Math.min(content.bottom, window.innerHeight) - content.top,
        ),
        viewport: window.innerHeight,
      };
    });
    expect(geometry).not.toBeNull();
    expect(
      geometry!.stripTop,
      `tab strip top is ${
        geometry!.stripTop
      }px; the broken build measured ${BROKEN_TABLIST_TOP_PX}px inside a ${
        geometry!.viewport
      }px viewport`,
    ).toBeLessThan(FOLD_LIMIT_PX);
    expect(
      geometry!.visibleContent,
      `the selected tab panel starts at ${geometry!.contentTop}px and only ${
        geometry!.visibleContent
      }px of it is inside the ${
        geometry!.viewport
      }px viewport; a tab strip above the fold with nothing readable under it is still an empty first screen`,
    ).toBeGreaterThanOrEqual(MIN_VISIBLE_CONTENT_PX);
  });

  test("mounts exactly one scene on Home and Play and none elsewhere", async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280 });

    for (const [tab, worlds] of [
      ["Reports", 0],
      ["Activity", 0],
      ["Settings", 0],
      ["Play", 1],
      ["Home", 1],
    ] as const) {
      await page.getByRole("tab", { name: tab, exact: true }).click();
      await expect(page.getByTestId("buddy-world")).toHaveCount(worlds);
      if (worlds) await expect(page.getByTestId("buddy-world")).toBeVisible();
    }
  });

  test("hides empty native panels instead of parking them above the fold", async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280 });
    await expect(page.getByLabel("Native work status")).toHaveCount(1);

    for (const label of NATIVE_PANEL_LABELS) {
      await expect(
        page.getByLabel(label),
        `${label} rendered with no decisions, approvals or mandate`,
      ).toHaveCount(0);
    }
  });

  test("renders each opportunity title exactly once", async ({ page }) => {
    await openBuddy(page, { width: 1280 });

    await expect(
      page.getByText(OPPORTUNITY_TITLE, { exact: true }),
    ).toHaveCount(0);

    await page.getByRole("tab", { name: "Reports", exact: true }).click();
    await page.getByTestId("buddy-opportunities-feed").waitFor();

    await expect(
      page.getByText(OPPORTUNITY_TITLE, { exact: true }),
      "the opportunity feed used to be duplicated by BuddyBriefingCard and the inbox overlay",
    ).toHaveCount(1);
  });

  test("reports a failed decisions or approvals request as an error, never as an empty state", async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280, fixture: "unavailable" });

    for (const label of NATIVE_PANEL_LABELS) {
      if (label === "Buddy supervision")
        await page.getByRole("tab", { name: "Activity", exact: true }).click();
      const panel = page.getByLabel(label);
      await expect(panel).toHaveCount(1);
      await expect(panel.getByRole("alert")).toHaveCount(1);
      await expect(
        panel.getByRole("button", { name: "Retry", exact: true }),
      ).toBeVisible();
    }

    for (const copy of REASSURING_EMPTY_COPY) {
      await expect(
        page.getByText(copy, { exact: false }),
        `503 rendered the reassuring copy "${copy}"`,
      ).toHaveCount(0);
    }
  });

  test("gives inert controls a visible disabled affordance", async ({
    page,
  }) => {
    await openBuddy(page, { width: 1280, fixture: "supervision-blocked" });
    await page.getByRole("tab", { name: "Activity", exact: true }).click();
    await page
      .getByRole("button", { name: "Grant a bounded mandate", exact: true })
      .click();

    const affordances = await page.evaluate(() =>
      [
        ...document.querySelectorAll(
          "fieldset[disabled], [data-disabled='true']",
        ),
      ].map((group) => {
        const style = getComputedStyle(group);
        const fields = [
          ...group.querySelectorAll<HTMLElement>(
            "input:disabled, select:disabled, textarea:disabled, button:disabled",
          ),
        ];
        return {
          label:
            group.getAttribute("aria-label") ?? group.tagName.toLowerCase(),
          opacity: Number.parseFloat(style.opacity),
          cursor: style.cursor,
          fieldCount: fields.length,
          fieldCursors: [
            ...new Set(fields.map((field) => getComputedStyle(field).cursor)),
          ],
        };
      }),
    );

    expect(
      affordances.length,
      "no inert control groups rendered, so the disabled affordance was not exercised",
    ).toBeGreaterThan(0);
    for (const group of affordances) {
      expect(group.fieldCount).toBeGreaterThan(0);
      expect(
        group.opacity < 1 || group.cursor === "not-allowed",
        `inert group "${group.label}" looks fully active: opacity ${group.opacity}, cursor ${group.cursor}`,
      ).toBe(true);
      expect(
        group.fieldCursors,
        `inert fields in "${group.label}" do not carry a not-allowed cursor`,
      ).toEqual(["not-allowed"]);
    }
  });
});
