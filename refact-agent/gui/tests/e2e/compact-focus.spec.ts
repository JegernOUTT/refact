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
