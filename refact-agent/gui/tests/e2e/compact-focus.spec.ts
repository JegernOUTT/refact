import { expect, test } from "@playwright/test";

test.use({ reducedMotion: "reduce" });

test("native inert protects nested closed details during real Tab and Shift+Tab", async ({
  page,
}) => {
  await page.goto("/tests/e2e/compact-focus.html");
  expect(await page.evaluate(() => "inert" in HTMLElement.prototype)).toBe(
    true,
  );
  const outer = page.getByRole("button", { name: "Outer receipt" });
  await outer.focus();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "After cards" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(outer).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(outer).toHaveAttribute("aria-expanded", "true");
  const inner = page.getByRole("button", { name: "More details" });
  const body = page.locator(
    `[id="${await inner.getAttribute("aria-controls")}"]`,
  );
  await expect(body).toHaveAttribute("inert", "");
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Related navigation" }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(inner).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "After cards" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(inner).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(
    page.getByRole("button", { name: "Related navigation" }),
  ).toBeFocused();
});

test("fallback alone preserves nested focus ownership without native inert", async ({
  page,
}) => {
  await page.goto("/tests/e2e/compact-focus.html?fallback");
  const outer = page.getByRole("button", { name: "Outer receipt" });
  await outer.focus();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "After cards" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(outer).toBeFocused();
  await page.keyboard.press("Enter");
  const inner = page.getByRole("button", { name: "More details" });
  const body = page.locator(
    `[id="${await inner.getAttribute("aria-controls")}"]`,
  );
  await expect(body).toHaveAttribute("data-test-inert", "");
  await expect(page.locator("[inert]")).toHaveCount(0);
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Related navigation" }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(inner).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "After cards" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(inner).toBeFocused();
  await expect(page.getByLabel("Notes")).toHaveAttribute("tabindex", "-1");
});

for (const mode of ["native", "fallback"]) {
  test(`${mode}: independent disclosure, paging, dynamic controls, rapid toggles and unmount`, async ({
    page,
  }) => {
    await page.goto(
      `/tests/e2e/compact-focus.html${mode === "fallback" ? "?fallback" : ""}`,
    );
    const outer = page.getByRole("button", { name: "Outer receipt" });
    const inner = page.getByRole("button", { name: "More details" });
    const notes = page.getByLabel("Notes");
    const after = page.getByRole("button", { name: "After cards" });
    await outer.click();
    await inner.click();
    await notes.fill("Still mounted");
    await page.getByRole("button", { name: "Next section" }).click();
    await expect(page.getByLabel("Full response content")).toHaveText("END");

    for (let index = 0; index < 3; index++) {
      await outer.focus();
      await page.keyboard.press("Enter");
      await expect(notes).toHaveAttribute("tabindex", "-1");
      await page.keyboard.press("Tab");
      await expect(after).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(outer).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(inner).toHaveAttribute("aria-expanded", "true");
      await expect(notes).toHaveValue("Still mounted");
      await expect(notes).not.toHaveAttribute("tabindex");
      await expect(page.getByLabel("Full response content")).toHaveText("END");
    }

    await inner.focus();
    await page.keyboard.press("Space");
    await page
      .getByRole("button", { name: "Add control", exact: true })
      .click();
    const added = page.getByRole("button", {
      name: "Added action",
      includeHidden: true,
    });
    await expect(added).toHaveAttribute("tabindex", "-1");
    await page.getByRole("button", { name: "Toggle outer externally" }).click();
    await expect(notes).toHaveAttribute("tabindex", "-1");
    await page.getByRole("button", { name: "Toggle outer externally" }).click();
    await expect(inner).toHaveAttribute("aria-expanded", "false");
    await expect(added).toHaveAttribute("tabindex", "-1");
    await inner.focus();
    await page.keyboard.press("Tab");
    await expect(after).toBeFocused();
    await page.keyboard.press("Shift+Tab");
    await expect(inner).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(added).not.toHaveAttribute("tabindex");
    await added.focus();
    await page.keyboard.press("Tab");
    await expect(after).toBeFocused();
    await page.keyboard.press("Shift+Tab");
    await expect(added).toBeFocused();

    await outer.click();
    const detachedNotes = await notes.elementHandle();
    const detachedAdded = await added.elementHandle();
    await page.getByRole("button", { name: "Unmount cards" }).click();
    expect(
      await detachedNotes?.evaluate((element) =>
        element.getAttribute("tabindex"),
      ),
    ).toBeNull();
    expect(
      await detachedAdded?.evaluate((element) =>
        element.getAttribute("tabindex"),
      ),
    ).toBeNull();
    await page.getByRole("button", { name: "Mount cards" }).click();
    await outer.click();
    await expect(inner).toHaveAttribute("aria-expanded", "false");
    await expect(notes).toHaveAttribute("tabindex", "-1");
    await detachedNotes?.dispose();
    await detachedAdded?.dispose();
  });
}

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
