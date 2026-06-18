import { expect, test } from "@playwright/test";

test.describe("workspace tab drag and drop", () => {
  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (error) => {
      throw error;
    });
  });

  test("dragging a chat tab onto an unsplit chat creates a two-pane group", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=chat-dnd");
    await page.locator("[data-workspace-unsplit-drop-target='true']").waitFor();

    const source = page.getByRole("tab", {
      name: /Responsive split pane companion/,
    });
    const target = page.locator("[data-workspace-unsplit-drop-target='true']");
    const dataTransfer = await page.evaluateHandle(() => new DataTransfer());

    await source.dispatchEvent("dragstart", { dataTransfer });
    await target.dispatchEvent("dragenter", { dataTransfer });
    await target.dispatchEvent("dragover", { dataTransfer });
    await target.dispatchEvent("drop", { dataTransfer });
    await source.dispatchEvent("dragend", { dataTransfer });

    await expect(
      page.locator("[data-workspace-group-tab-id='chat:showcase-chat']"),
    ).toBeVisible();
    await expect(page.locator("[data-workspace-leaf-id]")).toHaveCount(2);
    await expect(
      page.locator("[data-surface-key='chat:showcase-chat']"),
    ).toBeVisible();
    await expect(
      page.locator("[data-surface-key='chat:showcase-chat-b']"),
    ).toBeVisible();
    await expect(page.getByRole("tab")).toHaveCount(1);
    await expect(page.getByLabel("2 panes")).toBeVisible();
  });
});
