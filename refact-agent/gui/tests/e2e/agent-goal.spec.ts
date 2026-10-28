import { test, expect, type Page } from "@playwright/test";

async function openItem(page: Page, route: string, theme: string) {
  await page.goto(
    `/tests/e2e/route-showcase.html?route=${route}&theme=${theme}`,
  );
  await page.waitForLoadState("networkidle");
  const toggle = page.getByTestId("transcript-toggle").first();
  await expect(toggle).toBeVisible();
  if ((await toggle.getAttribute("aria-expanded")) === "false")
    await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByText("More details")).toHaveCount(0);
  await expect(page.getByTestId("transcript-raw")).toHaveCount(1);
  return page.locator("[data-transcript-item]").first();
}

async function expectNoOverflow(page: Page) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
}

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Goal row shows the goal markdown and usage facts at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      const item = await openItem(page, "agent-goal", theme);
      await expect(item.getByTestId("transcript-verb")).toHaveText("Goal");
      await expect(item.getByTestId("transcript-target")).toContainText(
        "Preserve the approved goal.",
      );
      await expect(item.getByTestId("transcript-facts")).toContainText(
        "Paused",
      );
      await expect(item.getByTestId("transcript-facts")).toContainText("3210");
      const body = item.getByTestId("agentic-tool-message");
      await expect(body).toContainText("Second goal section");
      await expect(body.locator("script")).toHaveCount(0);
      await expectNoOverflow(page);
    });
  }
}
