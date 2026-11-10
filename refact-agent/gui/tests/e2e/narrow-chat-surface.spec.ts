import { expect, test, type Page } from "@playwright/test";

async function expectChatsFillPanes(page: Page, panes: number) {
  const paneLocator = page.getByTestId("fixture-pane");
  await expect(paneLocator).toHaveCount(panes);
  for (let index = 0; index < panes; index += 1) {
    const pane = paneLocator.nth(index);
    const surface = pane.locator("[data-review]");
    await expect(surface).toHaveAttribute("data-review", "false");
    await expect(surface).toHaveAttribute("data-narrow", "true");
    const tracks = await surface.evaluate(
      (node) => getComputedStyle(node).gridTemplateColumns,
    );
    expect(tracks.split(" "), `surface columns: ${tracks}`).toHaveLength(1);
    const paneBox = await pane.boundingBox();
    const chatBox = await pane.getByTestId("chat-content").boundingBox();
    const composerBox = await pane.getByRole("textbox").boundingBox();
    expect(paneBox).not.toBeNull();
    expect(chatBox, "chat content has no layout box").not.toBeNull();
    expect(composerBox, "composer has no layout box").not.toBeNull();
    expect(chatBox!.width).toBeGreaterThanOrEqual(paneBox!.width * 0.9);
    expect(chatBox!.height).toBeGreaterThan(paneBox!.height * 0.5);
    expect(composerBox!.width).toBeGreaterThanOrEqual(300);
  }
}

test("chat fills a single narrow pane with no review open", async ({
  page,
}) => {
  await page.setViewportSize({ width: 560, height: 700 });
  await page.goto("/tests/e2e/narrow-chat-surface.html");
  await expectChatsFillPanes(page, 1);
});

test("chat fills both panes of a two-pane split with no review open", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1100, height: 700 });
  await page.goto("/tests/e2e/narrow-chat-surface.html?panes=2");
  await expectChatsFillPanes(page, 2);
});
