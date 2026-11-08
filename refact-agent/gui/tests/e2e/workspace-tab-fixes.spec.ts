import { expect, test, type Page } from "@playwright/test";
import type { setUpStore } from "../../src/app/store";

const fixture =
  "/tests/e2e/route-showcase.html?route=chat-dnd&toolbar_alignment&unsplit";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(fixture);
});

test("lower split header owns its tabs above its content, without top duplicates", async ({
  page,
}) => {
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
        dir: "col",
        surfaceKey: "chat:showcase-chat-b",
      },
    });
  });
  const panes = page.locator("[data-workspace-leaf-id]");
  await expect(panes).toHaveCount(2);
  const lower = panes.nth(1);
  await expect(lower.getByRole("tab")).toHaveCount(1);
  const lowerName = await lower.getByRole("tab").getAttribute("title");
  await expect(
    page.getByRole("tab").and(page.locator(`[title="${lowerName}"]`)),
  ).toHaveCount(1);
  await expect
    .poll(() =>
      lower.evaluate((pane) => {
        const header = pane
          .querySelector("[data-pane-tab-header]")!
          .getBoundingClientRect();
        const body = pane
          .querySelector('[role="tabpanel"]:not([hidden])')!
          .getBoundingClientRect();
        const rect = pane.getBoundingClientRect();
        return (
          Math.abs(header.left - rect.left) < 1 &&
          Math.abs(header.right - rect.right) <= 1 &&
          header.bottom <= body.top + 1
        );
      }),
    )
    .toBe(true);
  const tab = lower.getByRole("tab");
  const controls = await tab.getAttribute("aria-controls");
  await expect(page.locator(`[id="${controls}"]`)).toHaveAttribute(
    "role",
    "tabpanel",
  );
});

test("short tabs stay compact, undecorated and reserve a visible close even when narrow", async ({
  page,
}) => {
  const tabs = page.getByRole("tablist", {
    name: "Workspace tabs",
    exact: true,
  });
  await expect(tabs.getByRole("tab")).toHaveCount(4);
  for (const tab of await tabs.getByRole("tab").all()) {
    await expect(tab).toHaveCSS("text-decoration-line", "none");
    const close = tab.locator("..").getByRole("button");
    await expect(close).toHaveCSS("opacity", "1");
  }
  await page.setViewportSize({ width: 360, height: 800 });
  const selected = tabs.getByRole("tab", { selected: true });
  await expect(selected.locator("..").getByRole("button")).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => {
    const store = (
      window as unknown as {
        __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
      }
    ).__toolbarAlignmentStore;
    for (const key of [
      "chat:alignment-middle",
      "chat:alignment-last",
      "chat:showcase-chat-b",
    ])
      store.dispatch({ type: "workspace/closeTab", payload: key });
  });
  await expect(tabs.getByRole("tab")).toHaveCount(1);
  expect(
    await tabs
      .getByRole("tab")
      .locator("..")
      .evaluate((tab) => tab.getBoundingClientRect().width),
  ).toBeLessThan(180);
});

test("web middle click closes without browser popup or activation", async ({
  page,
  context,
}) => {
  const first = page.getByRole("tab", { name: /First chat$/ });
  const middle = page.getByRole("tab", { name: /Middle chat$/ });
  await first.click();
  const pages = context.pages().length;
  await middle.click({ button: "middle" });
  await expect(middle).toHaveCount(0);
  await expect(first).toHaveAttribute("aria-selected", "true");
  expect(context.pages()).toHaveLength(pages);
});

async function split(
  page: Page,
  surfaceKey: string,
  dir: "row" | "col" = "row",
  leafId?: string,
) {
  await page.evaluate(
    ({ surfaceKey, dir, leafId }) => {
      const store = (
        window as unknown as {
          __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
        }
      ).__toolbarAlignmentStore;
      store.dispatch({
        type: leafId ? "workspace/splitPaneWithSurface" : "workspace/splitTab",
        payload: { tabId: "chat:showcase-chat", surfaceKey, dir, leafId },
      });
    },
    { surfaceKey, dir, leafId },
  );
}

test("lower file actions do not overlap local tabs or close buttons", async ({
  page,
}) => {
  await split(page, "file:/workspace/example.ts", "col");
  const lower = page.locator("[data-workspace-leaf-id]").nth(1);
  const header = lower.locator("[data-pane-tab-header]");
  await expect(header.getByRole("tab")).toHaveCount(1);
  await lower.hover();
  const controls = lower.locator("[data-pane-controls]");
  await expect(controls.getByRole("button").first()).toBeVisible();
  await expect
    .poll(async () => {
      const h = await header.boundingBox();
      const c = await controls.boundingBox();
      return Boolean(h && c && c.y >= h.y + h.height);
    })
    .toBe(true);
  await expect(header.getByRole("button")).toBeVisible();
  await header.getByRole("button").click();
  await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(1);
});

test("focused selected tab and close survive toolbar to local header resize without stealing content focus", async ({
  page,
}) => {
  await split(page, "chat:showcase-chat-b");
  const panes = page.locator("[data-workspace-leaf-id]");
  const id = await panes.nth(1).getAttribute("data-workspace-leaf-id");
  const projected = page.locator(`[data-toolbar-pane-id="${id}"]`);
  const local = panes.nth(1).locator("[data-pane-tab-header]");
  await expect(projected.getByRole("tab")).toHaveCount(1);
  const resize = async (sizes: number[]) => {
    await page.evaluate((sizes) => {
      const store = (
        window as unknown as {
          __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
        }
      ).__toolbarAlignmentStore;
      const root = store.getState().workspace.groups["chat:showcase-chat"].root;
      store.dispatch({
        type: "workspace/resizeGroupSplit",
        payload: { tabId: "chat:showcase-chat", splitId: root.id, sizes },
      });
    }, sizes);
  };
  await projected.getByRole("tab").focus();
  await resize([0.97, 0.03]);
  await expect(local.getByRole("tab")).toBeFocused();
  await local.getByRole("button").focus();
  await resize([0.5, 0.5]);
  await expect(projected.getByRole("button")).toBeFocused();
  const content = panes
    .first()
    .locator('[contenteditable="true"], textarea')
    .first();
  await content.focus();
  await resize([0.97, 0.03]);
  await expect(local.getByRole("tab")).toHaveCount(1);
  await expect(content).toBeFocused();
});

for (const count of [4, 5]) {
  test(`${count} packed horizontal leaves and T split keep unique reachable per-pane headers`, async ({
    page,
  }) => {
    for (const key of [
      "chat:alignment-middle",
      "chat:alignment-last",
      "chat:showcase-chat-b",
      "file:/workspace/example.ts",
    ].slice(0, count - 1)) {
      await split(page, key);
    }
    const verify = async (expected = count) => {
      await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(
        expected,
      );
      await expect
        .poll(() =>
          page.evaluate(() => {
            return Array.from(
              document.querySelectorAll<HTMLElement>(
                "[data-workspace-leaf-id]",
              ),
            ).every((pane) => {
              const id = pane.dataset.workspaceLeafId;
              const header =
                document.querySelector<HTMLElement>(
                  `[data-toolbar-pane-id="${id}"]`,
                ) ?? pane.querySelector<HTMLElement>("[data-pane-tab-header]");
              if (
                !header ||
                header.querySelectorAll('[role="tab"]').length !== 1
              )
                return false;
              const h = header.getBoundingClientRect();
              const p = pane.getBoundingClientRect();
              const band = document
                .querySelector('[aria-label="Workspace toolbar tabs"]')!
                .getBoundingClientRect();
              return (
                h.width > 0 &&
                Math.abs(
                  h.left -
                    Math.max(
                      p.left,
                      header.hasAttribute("data-toolbar-pane-id")
                        ? band.left
                        : p.left,
                    ),
                ) <= 1 &&
                Math.abs(
                  h.right -
                    Math.min(
                      p.right,
                      header.hasAttribute("data-toolbar-pane-id")
                        ? band.right
                        : p.right,
                    ),
                ) <= 1
              );
            });
          }),
        )
        .toBe(true);
      const tabs = page.locator(
        '[data-pane-tab-key] [role="tab"], [role="tab"][data-pane-tab-key]',
      );
      await expect(tabs).toHaveCount(expected);
      const titles = await tabs.evaluateAll((nodes) =>
        nodes.map((node) => node.getAttribute("title")),
      );
      expect(new Set(titles).size).toBe(expected);
      for (const tab of await tabs.all()) {
        await tab.click();
        await expect(tab).toHaveAttribute("aria-selected", "true");
        await expect(tab.locator("..").getByRole("button")).toBeVisible();
      }
    };
    await verify();
    const last = page.locator("[data-workspace-leaf-id]").last();
    const leafId = await last.getAttribute("data-workspace-leaf-id");
    await page.evaluate(
      ({ leafId }) => {
        const store = (
          window as unknown as {
            __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
          }
        ).__toolbarAlignmentStore;
        const root =
          store.getState().workspace.groups["chat:showcase-chat"].root;
        if (root.kind !== "split") throw new Error("Expected horizontal split");
        store.dispatch({
          type: "workspace/splitPaneWithSurface",
          payload: {
            tabId: "chat:showcase-chat",
            leafId,
            surfaceKey: "file:/workspace/mixed.ts",
            dir: "col",
          },
        });
      },
      { leafId },
    );
    await expect(page.locator('[data-pane-split-dir="col"]')).toHaveCount(1);
    await verify(count + 1);
  });
}
