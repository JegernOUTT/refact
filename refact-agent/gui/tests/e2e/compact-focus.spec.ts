import { expect, test } from "@playwright/test";

test.use({ contextOptions: { reducedMotion: "reduce" } });

for (const mode of ["native", "fallback"]) {
  for (const row of [
    {
      kind: "status",
      firstRole: "tab",
      firstName: "All 1",
      actions: [
        "View pulse T-1",
        "View diff T-1",
        "Steer T-1",
        "Cancel agent T-1",
      ],
    },
    {
      kind: "pulse",
      firstRole: "button",
      firstName: "Steer",
      actions: ["Steer", "Pause", "Cancel", "Diff"],
    },
    {
      kind: "diff",
      firstRole: "button",
      firstName: "All files",
      actions: ["All files", "src/one.ts", "src/two.ts"],
    },
  ] as const) {
    test(`${mode}: dedicated ${row.kind} keeps closed inner controls outside real Tab order`, async ({
      page,
    }) => {
      const commandRequests: string[] = [];
      await page.route("**/v1/**", (route) => {
        if (new URL(route.request().url()).pathname.endsWith("/commands")) {
          commandRequests.push(route.request().url());
        }
        return route.abort();
      });
      await page.goto(
        `/tests/e2e/compact-focus.html?dedicated${
          mode === "fallback" ? "&fallback" : ""
        }`,
      );
      const scope = page.getByRole("region", {
        name: `Dedicated ${row.kind}`,
        exact: true,
      });
      const outer = scope.getByTestId("transcript-toggle");
      const after = scope.getByRole("button", {
        name: `After ${row.kind}`,
      });
      // Diff previews start expanded; close explicitly before testing Tab exclusion.
      if (row.kind === "diff") {
        await expect(outer).toHaveAttribute("aria-expanded", "true");
        await outer.focus();
        await page.keyboard.press("Space");
      }
      await expect(outer).toHaveAttribute("aria-expanded", "false");
      await outer.focus();
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(outer).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(outer).toHaveAttribute("aria-expanded", "true");
      const body = scope.locator(
        `[id="${await outer.getAttribute("aria-controls")}"]`,
      );
      await expect(body).toBeVisible();
      await expect(
        scope.getByRole("button", { name: "More details" }),
      ).toHaveCount(0);
      await page.keyboard.press("Tab");
      await expect(
        body.getByRole(row.firstRole, { name: row.firstName, exact: true }),
      ).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(outer).toBeFocused();
      if (row.kind === "status") {
        await body.getByRole("link", { name: "T-1", exact: true }).focus();
      }
      for (const name of row.actions) {
        const action = body.getByRole("button", { name, exact: true });
        await expect(action).toBeEnabled();
        await expect(action).not.toHaveAttribute("tabindex", "-1");
        await page.keyboard.press("Tab");
        await expect(action).toBeFocused();
      }
      const raw = body.getByRole("button", { name: "Raw", exact: true });
      await page.keyboard.press("Tab");
      await expect(raw).toBeFocused();
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(raw).toBeFocused();
      for (const name of [...row.actions].reverse()) {
        await page.keyboard.press("Shift+Tab");
        await expect(
          body.getByRole("button", { name, exact: true }),
        ).toBeFocused();
      }
      await outer.focus();
      await page.keyboard.press("Space");
      await expect(outer).toHaveAttribute("aria-expanded", "false");
      await expect(body).not.toBeVisible();
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(outer).toBeFocused();
      await page.keyboard.press("Enter");
      await page.keyboard.press("Tab");
      await expect(
        body.getByRole(row.firstRole, { name: row.firstName, exact: true }),
      ).toBeFocused();
      if (mode === "fallback")
        await expect(page.locator("[inert]")).toHaveCount(0);
      expect(commandRequests).toEqual([]);
    });
  }
}
