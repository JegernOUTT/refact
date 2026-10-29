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
  root: { scrollWidth: number; clientWidth: number };
  main: { scrollWidth: number; clientWidth: number };
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
  controls: Array<{
    name: string | null;
    left: number;
    right: number;
    width: number;
    top: number;
    bottom: number;
    area: number;
  }>;
  overflowingControlStrip: Array<{
    name: string | null;
    scrollWidth: number;
    clientWidth: number;
    scrollbarWidth: string;
  }>;
  textWidth: Array<{
    text: string;
    width: number;
    minimumWidth: number;
    overflowWrap: string;
    wordBreak: string;
  }>;
  brokenWords: string[];
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
    const identity = document.querySelector<HTMLElement>(
      '[data-testid="buddy-presence"] header > :first-child',
    );
    const textBlocks = identity
      ? [...identity.children].filter(
          (element): element is HTMLElement =>
            element instanceof HTMLElement && element.innerText.trim() !== "",
        )
      : [];
    const textWidth = textBlocks.map((element) => {
      const style = getComputedStyle(element);
      const canvas = document.createElement("canvas");
      const context = canvas.getContext("2d");
      if (context) context.font = style.font;
      return {
        text: element.innerText,
        width: element.getBoundingClientRect().width,
        minimumWidth: context?.measureText("M".repeat(12)).width ?? 0,
        overflowWrap: style.overflowWrap,
        wordBreak: style.wordBreak,
      };
    });
    const headline = identity?.children.item(1);
    const textNode = headline?.firstChild;
    const brokenWords: string[] = [];
    if (textNode?.nodeType === Node.TEXT_NODE) {
      const content = textNode.textContent ?? "";
      for (const match of content.matchAll(/\S+/g)) {
        const start = match.index;
        if (start === undefined) continue;
        const range = document.createRange();
        range.setStart(textNode, start);
        range.setEnd(textNode, start + match[0].length);
        if (range.getClientRects().length > 1) brokenWords.push(match[0]);
      }
    }
    const controls = [
      ...main!.querySelectorAll<HTMLElement>(
        'button, [role="tab"], select, input:not([type="radio"]), summary, a[href]',
      ),
    ]
      .filter(
        (element) =>
          element.getClientRects().length > 0 &&
          getComputedStyle(element).visibility !== "hidden",
      )
      .map((element) => ({
        name:
          element.getAttribute("aria-label") ||
          element.closest("label")?.innerText ||
          element.innerText,
        ...box(element),
      }));
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
      root: {
        scrollWidth: document.documentElement.scrollWidth,
        clientWidth: document.documentElement.clientWidth,
      },
      main: {
        scrollWidth: main!.scrollWidth,
        clientWidth: main!.clientWidth,
      },
      hero: hero ? box(hero) : null,
      tabs: {
        scrollWidth: tabs!.scrollWidth,
        clientWidth: tabs!.clientWidth,
        labels: [...tabs!.querySelectorAll<HTMLElement>('[role="tab"]')].map(
          (tab) => ({ text: tab.innerText, ...box(tab) }),
        ),
      },
      controls,
      overflowingControlStrip,
      textWidth,
      brokenWords,
    };
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`Buddy narrow layout at 360px in ${theme}`, () => {
    test.beforeEach(async ({ page }) => {
      await openBuddy(page, theme);
    });

    test("no_interactive_element_renders_off_screen", async ({ page }) => {
      for (const section of SECTIONS) {
        await page.getByRole("tab", { name: section, exact: true }).click();
        if (section === "Reports") await openReports(page);
        const layout = await measureLayout(page);
        expect(
          layout.controls.filter(
            (control) =>
              !SECTIONS.includes(control.name ?? "") &&
              (control.left < -1 || control.right > 360),
          ),
          section,
        ).toEqual([]);
      }
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

    test("no_text_collapses_into_a_ribbon", async ({ page }) => {
      const layout = await measureLayout(page);
      expect(layout.textWidth.length).toBeGreaterThan(0);
      for (const block of layout.textWidth) {
        expect(
          block.width,
          `presence text block is too narrow to read: ${block.text}`,
        ).toBeGreaterThanOrEqual(block.minimumWidth);
        expect(block.wordBreak).not.toBe("break-all");
      }
      expect(layout.brokenWords).toEqual([]);
    });

    test("document_has_no_horizontal_overflow_at_360", async ({ page }) => {
      for (const section of SECTIONS) {
        await page.getByRole("tab", { name: section, exact: true }).click();
        if (section === "Reports") await openReports(page);
        const layout = await measureLayout(page);
        expect(layout.root.scrollWidth, section).toBe(layout.root.clientWidth);
        expect(layout.main.scrollWidth, section).toBeLessThanOrEqual(
          layout.main.clientWidth,
        );
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

    test("wraps the legacy tab strip instead of clipping controls", async ({
      page,
    }) => {
      await openBuddy(page, theme, "disabled");

      const layout = await page.evaluate(() => {
        const tabs = document.querySelector<HTMLElement>('[role="tablist"]');
        const controls = [
          ...document.querySelectorAll<HTMLElement>(
            'main[data-testid="buddy-home-content"] button, main[data-testid="buddy-home-content"] [role="tab"]',
          ),
        ].filter(
          (element) =>
            element.getClientRects().length > 0 &&
            getComputedStyle(element).visibility !== "hidden",
        );
        return {
          scrollWidth: tabs!.scrollWidth,
          clientWidth: tabs!.clientWidth,
          labels: [...tabs!.querySelectorAll<HTMLElement>('[role="tab"]')].map(
            (tab) => ({
              text: tab.innerText,
              left: tab.getBoundingClientRect().left,
              right: tab.getBoundingClientRect().right,
            }),
          ),
          offscreenControls: controls
            .map((element) => ({
              name:
                element.getAttribute("aria-label") ||
                element.closest("label")?.innerText ||
                element.innerText,
              left: element.getBoundingClientRect().left,
              right: element.getBoundingClientRect().right,
            }))
            .filter((element) => element.left < -1 || element.right > 360),
          root: {
            scrollWidth: document.documentElement.scrollWidth,
            clientWidth: document.documentElement.clientWidth,
          },
        };
      });

      expect(layout.labels.map((tab) => tab.text)).toEqual(SECTIONS);
      expect(
        layout.offscreenControls.filter(
          (control) => !SECTIONS.includes(control.name),
        ),
      ).toEqual([]);
      expect(layout.root.scrollWidth).toBe(layout.root.clientWidth);
    });
  });
}
