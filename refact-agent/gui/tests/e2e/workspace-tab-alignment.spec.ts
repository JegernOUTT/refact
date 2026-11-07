import { expect, test, type Page } from "@playwright/test";
import type { setUpStore } from "../../src/app/store";

const fixture =
  "/tests/e2e/route-showcase.html?route=chat-dnd&toolbar_alignment";

async function expectPaneLayout(page: Page) {
  const panes = page.locator("[data-workspace-leaf-id]");
  await expect(panes).toHaveCount(2);
  await page.locator("[data-workspace-group-tab-id]").evaluate((group) => {
    group.scrollTop = 0;
  });
  await expect
    .poll(() =>
      panes.evaluateAll((elements) => {
        const toolbar = document.querySelector('[role="toolbar"]')!;
        const group = document.querySelector("[data-workspace-group-tab-id]")!;
        const body = getComputedStyle(group.parentElement!);
        return (
          Math.abs(
            elements[0].getBoundingClientRect().top -
              toolbar.getBoundingClientRect().bottom,
          ) < 1 &&
          body.paddingTop === "0px" &&
          body.paddingRight === "8px" &&
          body.paddingBottom === "8px" &&
          body.paddingLeft === "8px" &&
          elements.every((pane) => getComputedStyle(pane).boxShadow === "none")
        );
      }),
    )
    .toBe(true);
}

async function expectAlignedSections(page: Page) {
  await expectPaneLayout(page);
  for (const pane of await page.locator("[data-workspace-leaf-id]").all()) {
    await expect(pane).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  }
  await expect(page.locator("[data-toolbar-pane-id]")).toHaveCount(2);
  await expect
    .poll(() =>
      page.locator("[data-toolbar-pane-id]").evaluateAll((sections) => {
        const slot = document
          .querySelector('[aria-label="Workspace toolbar tabs"]')!
          .getBoundingClientRect();
        return sections.every((section) => {
          const pane = Array.from(
            document.querySelectorAll<HTMLElement>("[data-workspace-leaf-id]"),
          ).find(
            (candidate) =>
              candidate.dataset.workspaceLeafId ===
              (section as HTMLElement).dataset.toolbarPaneId,
          );
          if (!pane) return false;
          const actual = section.getBoundingClientRect();
          const expected = pane.getBoundingClientRect();
          return (
            Math.abs(actual.left - Math.max(slot.left, expected.left)) < 1 &&
            Math.abs(actual.right - Math.min(slot.right, expected.right)) < 1
          );
        });
      }),
    )
    .toBe(true);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
});

for (const width of [1280, 2466]) {
  for (const active of [0, 1, 2]) {
    test(`three tabs align with both panes at ${width}px, active ${active}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`${fixture}&active=${active}`);
      await expectAlignedSections(page);
      const tabs = page
        .getByRole("tablist", { name: "Workspace tabs", exact: true })
        .getByRole("tab");
      await expect(tabs).toHaveCount(3);
      await expect(tabs.nth(active)).toHaveAttribute("aria-selected", "true");
      const bounds = await tabs.evaluateAll((elements) =>
        elements.map((element) => element.getBoundingClientRect().toJSON()),
      );
      expect(
        bounds.every(
          (rect) =>
            rect.top === bounds[0].top && rect.bottom === bounds[0].bottom,
        ),
      ).toBe(true);
      expect(bounds[0].right).toBeLessThan(bounds[1].left);
      expect(bounds[1].right).toBeLessThan(bounds[2].left);
    });
  }

  test(`newly split toolbar aligns without divider or window resize at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await page.goto(`${fixture}&unsplit`);
    await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(1);
    await page.evaluate(() => {
      const store = (
        window as unknown as {
          __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
        }
      ).__toolbarAlignmentStore;
      store.dispatch({
        type: "workspace/splitTab",
        payload: {
          tabId: "chat:showcase-chat",
          dir: "row",
          surfaceKey: "chat:showcase-chat-b",
        },
      });
    });
    await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(2);
    await expectAlignedSections(page);
  });
}

test("narrow stacked panes retain a bounded fallback strip", async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto(fixture);
  await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(2);
  await expectPaneLayout(page);
  await expect(page.locator("[data-workspace-leaf-id]").first()).toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(page.locator("[data-toolbar-pane-id]")).toHaveCount(0);
  await expect(
    page.getByRole("tablist", { name: "Workspace tabs", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(360);
});

test("shipping toolbar keeps keyboard activation, close and overflow reachable with mixed surfaces", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(`${fixture}&active=0`);
  await expectAlignedSections(page);
  const tabs = page.getByRole("tablist", {
    name: "Workspace tabs",
    exact: true,
  });
  await tabs.getByRole("tab", { name: /First chat$/ }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(tabs.getByRole("tab", { name: /Middle chat$/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  const close = page.getByRole("button", {
    name: "Close Middle chat",
    exact: true,
  });
  await close.focus();
  await page.keyboard.press("Enter");
  await expect(tabs.getByRole("tab")).toHaveCount(2);
  await page.evaluate(() => {
    const store = (
      window as unknown as {
        __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
      }
    ).__toolbarAlignmentStore;
    store.dispatch({
      type: "workspace/addSurfaceToPane",
      payload: {
        tabId: "chat:showcase-chat",
        leafId: "alignment-right",
        surfaceKey: "file:/workspace/refact/README.md",
      },
    });
  });
  const filePane = page.locator('[data-workspace-leaf-id="alignment-right"]');
  await expect(
    filePane.getByRole("tabpanel", { name: "README.md" }),
  ).toBeVisible();
  const surfaceColor = await filePane.evaluate((pane) => {
    const probe = document.createElement("div");
    probe.style.background = "var(--rf-surface-1)";
    pane.append(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  });
  expect(surfaceColor).not.toBe("rgba(0, 0, 0, 0)");
  await expect(filePane).toHaveCSS("background-color", surfaceColor);
  await expect(
    page.locator('[data-workspace-leaf-id="alignment-left"]'),
  ).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await filePane.hover();
  await expect(
    filePane.getByRole("button", { name: "File information", exact: true }),
  ).toBeVisible();
  await expect(filePane.locator("[data-pane-controls]")).toBeVisible();
  await expect(
    filePane.getByRole("button", { name: "Pane actions", exact: true }),
  ).toHaveCount(0);
  await expectPaneLayout(page);
  await page.setViewportSize({ width: 360, height: 800 });
  await expectPaneLayout(page);
  await expect(filePane).toHaveCSS("background-color", surfaceColor);
  const overflow = page.getByRole("button", { name: /more pane tabs/ }).first();
  await expect(overflow).toBeVisible();
  await overflow.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("menuitem").first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(overflow).toBeFocused();
});

test("floating pane menus stay absent while tab drag splitting and closing work", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(`${fixture}&unsplit`);
  const panes = page.locator("[data-workspace-leaf-id]");
  await expect(panes).toHaveCount(1);
  await panes.first().hover();
  await expect(
    page.getByRole("button", { name: "Pane actions", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Split active tab", exact: true }),
  ).toHaveCount(0);
  await expect(panes.first().locator("[data-pane-controls]")).toBeHidden();
  const paneBounds = await panes.first().boundingBox();
  if (!paneBounds) throw new Error("Missing pane bounds");
  await page.getByRole("tab", { name: /Middle chat$/ }).dragTo(panes.first(), {
    targetPosition: { x: paneBounds.width - 16, y: paneBounds.height / 2 },
  });
  await expectAlignedSections(page);
  await expect(
    page.getByRole("button", { name: "Pane actions", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Close Middle chat", exact: true })
    .click();
  await expect(page.getByRole("tab", { name: /Middle chat$/ })).toHaveCount(0);
  await expect(panes).toHaveCount(1);
});
