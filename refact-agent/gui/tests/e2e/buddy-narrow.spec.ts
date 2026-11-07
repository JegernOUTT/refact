import { expect, test, type Page } from "@playwright/test";

const PROJECT = { project_id: "showcase", canonical_root: "/workspace/refact" };
const WORK = {
  value: {
    schema_version: 1,
    id: "narrow-layout-check",
    project: PROJECT,
    subject: {
      resource_kind: "workspace",
      resource_id: "narrow-layout-check",
      version: "1",
    },
    kind: "assessment",
    title: "Check narrow Buddy layout",
    state: "needs_attention",
    native_sources: [],
    current_artifact_id: null,
    current_artifact_revision: null,
    last_material_change_at: new Date().toISOString(),
  },
  revision: 1,
  change_seq: 1,
};

type Layout = {
  hero: {
    left: number;
    right: number;
    width: number;
    top: number;
    bottom: number;
    area: number;
  } | null;
  tabs: {
    scrollWidth: number;
    clientWidth: number;
    labels: Array<{
      text: string;
      left: number;
      right: number;
      width: number;
      top: number;
      bottom: number;
      area: number;
    }>;
  };
  overflowingControlStrip: Array<{
    name: string | null;
    scrollWidth: number;
    clientWidth: number;
    scrollbarWidth: string;
  }>;
};

async function openBuddy(
  page: Page,
  theme: "dark" | "light",
  operator: "enabled" | "disabled" = "enabled",
) {
  await page.route("**/v1/buddy/work*", (route) =>
    route.fulfill({
      json: {
        schema_version: 1,
        project: PROJECT,
        data: [WORK],
        next_cursor: null,
      },
    }),
  );
  await page.setViewportSize({ width: 360, height: 900 });
  await page.goto(
    `/tests/e2e/route-showcase.html?route=buddy&operator=${operator}&theme=${theme}&scene_speech=1`,
  );
  await page.getByTestId("buddy-home-content").waitFor();
}

async function openReports(page: Page) {
  await page.getByRole("tab", { name: "Reports", exact: true }).click();
  await expect(
    page.locator('[data-testid="buddy-desk"] ul li button'),
  ).toHaveCount(1);
}

const SECTIONS = ["Home", "Reports", "Activity", "Play", "Settings"];

async function measureLayout(page: Page): Promise<Layout> {
  return page.evaluate(() => {
    const main = document.querySelector<HTMLElement>(
      'main[data-testid="buddy-home-content"]',
    );
    const hero = [
      ...document.querySelectorAll<HTMLElement>(
        '[data-testid="buddy-home-hero"]',
      ),
    ].find((element) => element.getClientRects().length > 0);
    const tabs = document.querySelector<HTMLElement>('[role="tablist"]');
    const box = (element: HTMLElement) => {
      const rect = element.getBoundingClientRect();
      return {
        left: rect.left,
        right: rect.right,
        width: rect.width,
        top: rect.top,
        bottom: rect.bottom,
        area: rect.width * rect.height,
      };
    };
    const overflowingControlStrip = [
      ...main!.querySelectorAll<HTMLElement>("*"),
    ]
      .filter((element) => {
        const style = getComputedStyle(element);
        return (
          element.getClientRects().length > 0 &&
          element.querySelector('button, [role="tab"]') !== null &&
          ["auto", "scroll"].includes(style.overflowX) &&
          element.scrollWidth > element.clientWidth + 1
        );
      })
      .map((element) => ({
        name:
          element.getAttribute("aria-label") || element.getAttribute("role"),
        scrollWidth: element.scrollWidth,
        clientWidth: element.clientWidth,
        scrollbarWidth: getComputedStyle(element).scrollbarWidth,
      }));
    return {
      hero: hero ? box(hero) : null,
      tabs: {
        scrollWidth: tabs!.scrollWidth,
        clientWidth: tabs!.clientWidth,
        labels: [...tabs!.querySelectorAll<HTMLElement>('[role="tab"]')].map(
          (tab) => ({ text: tab.innerText, ...box(tab) }),
        ),
      },
      overflowingControlStrip,
    };
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`Buddy narrow layout at 360px in ${theme}`, () => {
    test.beforeEach(async ({ page }) => {
      await openBuddy(page, theme);
    });

    test("no_control_strip_is_clipped_without_an_affordance", async ({
      page,
    }) => {
      const notSections = (strips: Layout["overflowingControlStrip"]) =>
        strips.filter((strip) => strip.name !== "Buddy sections");
      const layout = await measureLayout(page);
      expect(notSections(layout.overflowingControlStrip)).toEqual([]);
      await openReports(page);
      expect(
        notSections((await measureLayout(page)).overflowingControlStrip),
      ).toEqual([]);
      expect(layout.tabs.labels.map((tab) => tab.text)).toEqual(SECTIONS);
      for (const name of SECTIONS) {
        const tab = page.getByRole("tab", { name, exact: true });
        await tab.focus();
        await expect(tab).toBeFocused();
        const box = await tab.boundingBox();
        expect(box, name).not.toBeNull();
        expect(box!.x, name).toBeGreaterThanOrEqual(-1);
        expect(box!.x + box!.width, name).toBeLessThanOrEqual(361);
      }
    });

    test("keeps the Home scene compact and the full scene on Play only", async ({
      page,
    }) => {
      const home = await measureLayout(page);
      expect(home.hero).not.toBeNull();
      await openReports(page);
      await expect(page.getByTestId("buddy-world")).toHaveCount(0);
      await page.getByRole("tab", { name: "Play", exact: true }).click();
      const play = await measureLayout(page);
      expect(play.hero).not.toBeNull();
      expect(home.hero!.area).toBeLessThan(play.hero!.area);
      expect(home.hero!.right).toBeLessThanOrEqual(360);
    });
  });
}
