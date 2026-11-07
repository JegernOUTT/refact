import { test, expect } from "@playwright/test";
test.setTimeout(60000);
test.use({ actionTimeout: 30000 });
const check = expect.configure({ timeout: 30000 });

for (const width of [1280, 2466, 360]) {
  test(`loaded embedded surfaces fill their pane at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    for (const surface of ["buddy", "task"]) {
      await page.goto(`/tests/e2e/workspace-embedded.html?surface=${surface}`);
      const pane = page.locator(`[data-surface-key^="${surface}:"]`).last();
      await check(pane).toBeVisible();
      if (surface === "task") {
        await check(
          pane.getByRole("tab", { name: "Chat", exact: true }),
        ).toBeVisible();
        await pane.getByRole("tab", { name: "Chat", exact: true }).click();
        await check(pane.locator("textarea")).toBeVisible();
      } else
        await check(
          pane.getByText("Pixel", { exact: true }).first(),
        ).toBeVisible();
      const receipt = await pane.evaluate((node) => ({
        pane: node.getBoundingClientRect().width,
        root: node.firstElementChild!.getBoundingClientRect().width,
        scroll: document.documentElement.scrollWidth,
        viewport: innerWidth,
        composer: node.querySelector("textarea")?.getBoundingClientRect().width,
      }));
      console.log(surface, width, receipt);
      expect(Math.abs(receipt.root - receipt.pane)).toBeLessThan(2);
      expect(receipt.scroll).toBeLessThanOrEqual(receipt.viewport);
      if (receipt.composer)
        expect(receipt.composer).toBeGreaterThan(receipt.pane * 0.45);
    }
  });
}

test("split loaded Task stays bounded and fills its allocated pane", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/tests/e2e/workspace-embedded.html?surface=task&split=1");
  const pane = page.locator('[data-surface-key="task:embedded"]').last();
  await pane.getByRole("tab", { name: "Chat", exact: true }).click();
  await check(pane.locator("textarea")).toBeVisible();
  const widths = await pane.evaluate((node) => [
    node.clientWidth,
    node.firstElementChild!.clientWidth,
  ]);
  console.log("split", widths);
  expect(widths[0]).toBeLessThan(1000);
  expect(Math.abs(widths[0] - widths[1])).toBeLessThan(2);
});
