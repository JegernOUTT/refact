import { test, expect } from "@playwright/test";

for (const width of [360, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`Launch preview sources stay bounded and keyboard accessible at ${width}px in ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/route-showcase.html?route=agent-launch-preview&theme=${theme}`,
      );
      const preview = page.getByLabel("Child launch preview", { exact: true });
      await expect(preview).toBeVisible();
      await expect(preview).toContainText("No child spawned");
      await expect(preview).toContainText(
        "Unresolved requested context prevents launch",
      );
      await expect(
        preview.getByLabel("Launch context source", { exact: true }),
      ).toContainText("src/accepted-context.rs");
      const next = preview.getByRole("button", { name: "Next source" });
      await next.focus();
      await expect(next).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(
        preview.getByLabel("Launch context source", { exact: true }),
      ).toContainText("completed-parent-message");
      await expect(next).toBeDisabled();
      const authority = preview.getByLabel("Preview tool authority");
      await expect(authority).toContainText("cat");
      const nextTool = authority.getByRole("button", { name: "Next tool" });
      await nextTool.focus();
      await expect(nextTool).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(authority).toContainText("mcp_fixture_read");
      await expect(nextTool).toBeDisabled();
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true);
    });
  }
}
