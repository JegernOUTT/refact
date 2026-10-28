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
    test(`Launch preview sources stay keyboard accessible at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      const item = await openItem(page, "agent-launch-preview", theme);
      await expect(item.getByTestId("transcript-verb")).toHaveText("Launch");
      await expect(item.getByTestId("transcript-status-pill")).toHaveText(
        "Blocked",
      );
      const preview = item.getByLabel("Child launch preview", { exact: true });
      const source = preview.getByLabel("Launch context source", {
        exact: true,
      });
      await expect(source).toContainText("src/accepted-context.rs");
      const next = preview.getByRole("button", { name: "Next source" });
      await next.focus();
      await page.keyboard.press("Enter");
      await expect(source).toContainText("completed-parent-message");
      await expect(next).toBeDisabled();
      const authority = preview.getByLabel("Preview tool authority");
      const nextTool = authority.getByRole("button", { name: "Next tool" });
      await nextTool.focus();
      await page.keyboard.press("Enter");
      await expect(authority).toContainText("mcp_fixture_read");
      await expect(nextTool).toBeDisabled();
      await expectNoOverflow(page);
    });
  }
}
