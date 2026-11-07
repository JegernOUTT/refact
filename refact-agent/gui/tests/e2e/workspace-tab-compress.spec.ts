import { expect, test, type Page } from "@playwright/test";
import type { setUpStore } from "../../src/app/store";

const fixture =
  "/tests/e2e/route-showcase.html?route=chat-dnd&toolbar_alignment&unsplit";

const titles: Record<string, string> = {
  "showcase-chat": "Investigate flaky websocket reconnect handling",
  "alignment-middle": "Refactor billing invoice reconciliation pipeline",
  "alignment-last": "Document onboarding flow for enterprise tenants",
  "showcase-chat-b": "Audit dependency licenses across every package",
};

type Snapshot = {
  tabs: {
    title: string;
    selected: boolean;
    left: number;
    right: number;
    truncated: boolean;
    titleWidth: number;
    background: string;
    color: string;
  }[];
  stripRight: number;
  overflow: number;
  labelVisible: boolean;
  dots: number;
  pageOverflow: boolean;
};

async function open(page: Page, width: number) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(fixture);
  await page.evaluate((names) => {
    const store = (
      window as unknown as {
        __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
      }
    ).__toolbarAlignmentStore;
    for (const [id, title] of Object.entries(names)) {
      store.dispatch({
        type: "chatThread/saveTitle",
        payload: { id, title, isTitleGenerated: true },
      });
    }
  }, titles);
  await page.mouse.move(0, 899);
  await expect(workspaceTabs(page).getByRole("tab")).toHaveCount(
    Object.keys(titles).length,
  );
  for (const title of Object.values(titles)) {
    await expect(
      page
        .getByRole("tablist", { name: "Workspace tabs", exact: true })
        .locator(`[title="${title}"]`),
    ).toHaveCount(1);
  }
}

function workspaceTabs(page: Page) {
  return page.getByRole("tablist", { name: "Workspace tabs", exact: true });
}

function snapshot(page: Page): Promise<Snapshot> {
  return page.evaluate(() => {
    const list = document.querySelector<HTMLElement>(
      '[role="tablist"][aria-label="Workspace tabs"]',
    )!;
    const tabs = Array.from(
      list.querySelectorAll<HTMLElement>('[role="tab"]'),
    ).map((button) => {
      const title = Array.from(button.querySelectorAll("span")).find(
        (span) => span.textContent === button.title,
      )!;
      const rect = button.parentElement!.getBoundingClientRect();
      const style = getComputedStyle(button.parentElement!);
      return {
        title: button.title,
        selected: button.getAttribute("aria-selected") === "true",
        left: rect.left,
        right: rect.right,
        truncated:
          title.scrollWidth > title.clientWidth + 0.5 &&
          rect.width < parseFloat(style.maxWidth) - 0.5,
        titleWidth: title.getBoundingClientRect().width,
        background: style.backgroundColor,
        color: getComputedStyle(title).color,
      };
    });
    const overflowButton = Array.from(
      list.parentElement!.querySelectorAll<HTMLElement>("button"),
    ).find(
      (button) =>
        /more pane tabs$/.test(button.getAttribute("aria-label") ?? "") &&
        button.checkVisibility(),
    );
    const chip = document.querySelector<HTMLElement>(
      '[role="toolbar"] button[aria-label="Engine status"]',
    )!;
    const spans = Array.from(chip.querySelectorAll<HTMLElement>("span"));
    const label = spans.find((span) => (span.textContent ?? "").trim())!;
    const dots = spans.filter((span) => {
      const rect = span.getBoundingClientRect();
      return (
        span !== label &&
        rect.width > 0 &&
        rect.height > 0 &&
        span.checkVisibility({ visibilityProperty: true })
      );
    }).length;
    return {
      tabs,
      stripRight: list.getBoundingClientRect().right,
      overflow: overflowButton
        ? Number(
            /^(\d+)/.exec(overflowButton.getAttribute("aria-label") ?? "")?.[1],
          )
        : 0,
      labelVisible:
        label.checkVisibility({ visibilityProperty: true }) &&
        label.getBoundingClientRect().width > 0,
      dots,
      pageOverflow:
        document.documentElement.scrollWidth >
        document.documentElement.clientWidth,
    };
  });
}

async function settled(page: Page) {
  let previous = "";
  await expect
    .poll(async () => {
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );
      const next = JSON.stringify(await snapshot(page));
      const stable = next === previous;
      previous = next;
      return stable;
    })
    .toBe(true);
  return JSON.parse(previous) as Snapshot;
}

function expectEngineMatchesPressure(state: Snapshot, width: number) {
  expect(state.dots, `two status dots at ${width}px`).toBe(2);
  if (state.overflow > 0 || state.tabs.some((tab) => tab.truncated)) {
    expect(
      state.labelVisible,
      `label hidden under pressure at ${width}px`,
    ).toBe(false);
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
});

test("shrinking ellipsizes every tab before overflowing and compacts the engine chip", async ({
  page,
}) => {
  await open(page, 1600);
  const wide = await settled(page);
  expect(wide.overflow).toBe(0);
  expect(wide.tabs.some((tab) => tab.truncated)).toBe(false);
  expect(wide.labelVisible).toBe(true);
  expect(wide.dots).toBe(2);

  let ellipsizedWithAllVisible = false;
  let firstOverflowWidth = 0;
  for (const width of [
    1600, 1400, 1200, 1100, 1000, 900, 800, 700, 600, 500, 400, 360, 320, 240,
  ]) {
    await page.setViewportSize({ width, height: 900 });
    const state = await settled(page);
    expect(state.pageOverflow, `page scroll at ${width}px`).toBe(false);
    expect(state.tabs.length + state.overflow).toBe(4);
    expect(state.tabs.filter((tab) => tab.selected)).toHaveLength(1);
    for (const tab of state.tabs) {
      expect(
        tab.right,
        `${tab.title} clipped at ${width}px`,
      ).toBeLessThanOrEqual(state.stripRight + 0.5);
    }
    expectEngineMatchesPressure(state, width);
    if (state.overflow === 0 && state.tabs.some((tab) => tab.truncated)) {
      ellipsizedWithAllVisible = true;
      const widths = state.tabs.map((tab) => tab.right - tab.left);
      expect(Math.max(...widths) - Math.min(...widths)).toBeLessThanOrEqual(1);
      for (const tab of state.tabs)
        expect(tab.titleWidth, `${tab.title} at ${width}px`).toBeGreaterThan(0);
    }
    if (state.overflow > 0 && !firstOverflowWidth) {
      firstOverflowWidth = width;
      expect(ellipsizedWithAllVisible).toBe(true);
    }
  }
  expect(ellipsizedWithAllVisible).toBe(true);
  expect(firstOverflowWidth).toBeGreaterThan(0);

  await page.setViewportSize({ width: 1600, height: 900 });
  const restored = await settled(page);
  expect(restored.overflow).toBe(0);
  expect(restored.tabs.some((tab) => tab.truncated)).toBe(false);
  expect(restored.labelVisible).toBe(true);
});

test("compact engine chip stays stable across nearby resizes and frames, then restores", async ({
  page,
}) => {
  await open(page, 1600);
  let threshold = 0;
  for (let width = 1600; width >= 400; width -= 10) {
    await page.setViewportSize({ width, height: 900 });
    const state = await settled(page);
    expectEngineMatchesPressure(state, width);
    if (!state.labelVisible) {
      threshold = width;
      break;
    }
  }
  expect(threshold).toBeGreaterThan(400);

  for (const width of [
    threshold,
    threshold - 1,
    threshold - 3,
    threshold - 2,
    threshold - 6,
    threshold - 4,
  ]) {
    await page.setViewportSize({ width, height: 900 });
    const frames = await page.evaluate(
      () =>
        new Promise<boolean[]>((resolve) => {
          const samples: boolean[] = [];
          const sample = () => {
            const chip = document.querySelector<HTMLElement>(
              '[role="toolbar"] button[aria-label="Engine status"]',
            )!;
            const label = Array.from(chip.querySelectorAll("span")).find(
              (span) => (span.textContent ?? "").trim(),
            )!;
            samples.push(
              label.checkVisibility({ visibilityProperty: true }) &&
                label.getBoundingClientRect().width > 0,
            );
            if (samples.length < 20) requestAnimationFrame(sample);
            else resolve(samples);
          };
          requestAnimationFrame(sample);
        }),
    );
    expect(frames.slice(2), `label flicker at ${width}px`).toEqual(
      Array(18).fill(false),
    );
    const state = await settled(page);
    expect(state.labelVisible).toBe(false);
    expect(state.dots).toBe(2);
    expect(state.pageOverflow).toBe(false);
  }

  await page.setViewportSize({ width: 1600, height: 900 });
  const restored = await settled(page);
  expect(restored.labelVisible).toBe(true);
  expect(restored.tabs.some((tab) => tab.truncated)).toBe(false);
  expect(restored.overflow).toBe(0);
});

test("only the selected tab is transparent and highlighted, and narrow overflow stays keyboard reachable", async ({
  page,
}) => {
  await open(page, 1280);
  const tabs = workspaceTabs(page).getByRole("tab");
  const transparent = "rgba(0, 0, 0, 0)";
  const appearance = async () => {
    await page.mouse.move(0, 899);
    const state = await settled(page);
    const active = state.tabs.filter((tab) => tab.selected);
    const inactive = state.tabs.filter((tab) => !tab.selected);
    expect(active).toHaveLength(1);
    expect(inactive.length).toBeGreaterThan(0);
    expect(active[0].background).toBe(transparent);
    for (const tab of inactive) {
      expect(tab.background, tab.title).not.toBe(transparent);
      expect(tab.background, tab.title).toBe(inactive[0].background);
      expect(tab.color, tab.title).toBe(inactive[0].color);
      expect(tab.color, tab.title).not.toBe(active[0].color);
    }
    return active[0].title;
  };

  const first = await appearance();
  await tabs.and(page.locator(`[title="${first}"]`)).focus();
  await page.keyboard.press("ArrowRight");
  const second = await appearance();
  expect(second).not.toBe(first);

  await page.setViewportSize({ width: 360, height: 800 });
  const narrow = await settled(page);
  expect(narrow.pageOverflow).toBe(false);
  expect(narrow.overflow).toBeGreaterThan(0);
  const selected = workspaceTabs(page).getByRole("tab", { selected: true });
  await expect(selected).toBeVisible();
  await expect(selected).toHaveAccessibleName(new RegExp(second));
  const visible = new Set(narrow.tabs.map((tab) => tab.title));
  const hiddenTitle = Object.values(titles).find(
    (title) => !visible.has(title),
  )!;
  await selected.focus();
  for (let step = 0; step < 4; step++) {
    const name = await selected.getAttribute("title");
    if (name === hiddenTitle) break;
    await page.keyboard.press("ArrowRight");
  }
  const reached = workspaceTabs(page).getByRole("tab", { selected: true });
  await expect(reached).toHaveAttribute("title", hiddenTitle);
  await expect(reached).toBeVisible();
  await expect(reached).toBeFocused();

  // Activate a still-hidden chat through the portal menu, not arrow navigation.
  const overflow = page.getByRole("button", { name: /more pane tabs$/ });
  await overflow.focus();
  await page.keyboard.press("Enter");
  const item = page.getByRole("menuitem").first();
  const menuTitle = await item.getAttribute("title");
  await item.focus();
  await page.keyboard.press("Enter");
  const menuSelected = workspaceTabs(page).getByRole("tab", { selected: true });
  await expect(menuSelected).toHaveAttribute("title", menuTitle!);
  await expect(menuSelected).toBeFocused();
  // Focus must survive the new chat composer's native autofocus commit.
  await settled(page);
  await expect(menuSelected).toBeFocused();
  const final = await settled(page);
  const shown = final.tabs.filter((tab) => tab.selected);
  expect(shown).toHaveLength(1);
  expect(shown[0].title).toBe(menuTitle);
  expect(shown[0].background).toBe(transparent);
  expect(shown[0].right).toBeLessThanOrEqual(final.stripRight + 0.5);
});
