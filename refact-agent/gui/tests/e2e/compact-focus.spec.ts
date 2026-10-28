import { expect, test } from "@playwright/test";

test.use({ reducedMotion: "reduce" });

for (const mode of ["native", "fallback"]) {
  for (const row of [
    {
      kind: "status",
      title: "1 agent rows shown Snapshot received",
      firstRole: "tab",
      firstName: "All 1",
      beforeActions: "Toggle details T-1",
      actions: [
        "View pulse T-1",
        "View diff T-1",
        "Steer T-1",
        "Cancel agent T-1",
      ],
    },
    {
      kind: "pulse",
      title: "Inspect Focus inspection Agent paused",
      firstRole: "button",
      firstName: "Steer",
      beforeActions: "More details",
      actions: ["Steer", "Pause", "Cancel", "Diff"],
    },
    {
      kind: "diff",
      title: "2 changed files shown · Focus inspection Diff received",
      firstRole: "button",
      firstName: "All files",
      beforeActions: "More details",
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
      const outer = scope.getByRole("button", { name: row.title, exact: true });
      const after = scope.getByRole("button", {
        name: `After ${row.kind}`,
      });
      await expect(outer).toHaveAttribute("aria-expanded", "false");
      await outer.focus();
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(outer).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(outer).toHaveAttribute("aria-expanded", "true");
      const inner = scope.getByRole("button", { name: "More details" });
      await expect(inner).toHaveAttribute("aria-expanded", "false");
      const body = scope.locator(
        `[id="${await inner.getAttribute("aria-controls")}"]`,
      );
      await expect(body).toHaveAttribute(
        mode === "fallback" ? "data-test-inert" : "inert",
        "",
      );
      if (mode === "fallback") {
        await expect(page.locator("[inert]")).toHaveCount(0);
      }
      for (const name of row.actions) {
        await expect(
          body.getByRole("button", { name, exact: true, includeHidden: true }),
        ).toHaveAttribute("tabindex", "-1");
      }
      await page.keyboard.press("Tab");
      await expect(inner).toBeFocused();
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(inner).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(inner).toHaveAttribute("aria-expanded", "true");
      await page.keyboard.press("Tab");
      await expect(
        body.getByRole(row.firstRole, { name: row.firstName, exact: true }),
      ).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(inner).toBeFocused();
      await scope
        .getByRole("button", { name: row.beforeActions, exact: true })
        .focus();
      for (const name of row.actions) {
        const action = body.getByRole("button", { name, exact: true });
        await expect(action).toBeEnabled();
        await expect(action).not.toHaveAttribute("tabindex", "-1");
        await page.keyboard.press("Tab");
        await expect(action).toBeFocused();
      }
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      for (const name of [...row.actions].reverse()) {
        await page.keyboard.press("Shift+Tab");
        await expect(
          body.getByRole("button", { name, exact: true }),
        ).toBeFocused();
      }
      await inner.focus();
      await page.keyboard.press("Space");
      await expect(inner).toHaveAttribute("aria-expanded", "false");
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(inner).toBeFocused();
      await outer.focus();
      await page.keyboard.press("Enter");
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(outer).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(inner).toHaveAttribute("aria-expanded", "false");
      for (const name of row.actions) {
        await expect(
          body.getByRole("button", { name, exact: true, includeHidden: true }),
        ).toHaveAttribute("tabindex", "-1");
      }
      await inner.focus();
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(inner).toBeFocused();
      expect(commandRequests).toEqual([]);
    });
  }
}
