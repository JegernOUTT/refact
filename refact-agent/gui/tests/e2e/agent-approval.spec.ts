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
    test(`Approval row lists every request at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      const item = await openItem(page, "agent-approval", theme);
      const card = item.getByLabel("Child command approval");
      await expect(card).toContainText("first-request");
      await expect(card).toContainText("second-request");
      await expect(card).toContainText("consumed");
      await expectNoOverflow(page);
    });
  }
}
