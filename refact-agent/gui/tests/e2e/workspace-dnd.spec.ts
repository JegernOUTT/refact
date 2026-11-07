import { expect, test } from "@playwright/test";

test.describe("workspace tab drag and drop", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("dragging a chat tab to the pane edge creates a two-pane group", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=chat-dnd");
    const pane = page.locator("[data-workspace-leaf-id='root']");
    await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(1);
    const source = page.getByRole("tab", {
      name: /Responsive split pane companion/,
    });
    const sourceBox = await source.boundingBox();
    const paneBox = await pane.boundingBox();
    if (!sourceBox || !paneBox) throw new Error("Missing drag source or pane");

    await page.mouse.move(
      sourceBox.x + sourceBox.width / 2,
      sourceBox.y + sourceBox.height / 2,
    );
    await page.mouse.down();
    await page.mouse.move(
      paneBox.x + paneBox.width - 20,
      paneBox.y + paneBox.height / 2,
      { steps: 12 },
    );
    await expect(
      page.getByTestId("workspace-pane-drop-preview"),
    ).toHaveAttribute("data-drop-target", "right");
    await page.mouse.up();

    await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(2);
    await expect(
      page.locator("[data-surface-key='chat:showcase-chat']"),
    ).toBeVisible();
    await expect(
      page.locator("[data-surface-key='chat:showcase-chat-b']"),
    ).toBeVisible();
    await expect(page.getByRole("tab")).toHaveCount(2);
    // Both surfaces remain owned by distinct panes after the real drop.
    await expect(
      page.locator("[data-workspace-leaf-id]").filter({
        has: page.locator("[data-surface-key='chat:showcase-chat']"),
      }),
    ).toHaveCount(1);
    await expect(
      page.locator("[data-workspace-leaf-id]").filter({
        has: page.locator("[data-surface-key='chat:showcase-chat-b']"),
      }),
    ).toHaveCount(1);
  });
});
