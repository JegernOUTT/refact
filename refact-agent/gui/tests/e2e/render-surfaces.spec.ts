import { expect, test, type Page } from "@playwright/test";

const widths = [240, 320, 360, 480, 640, 768, 1280];
const themes = ["light", "dark"] as const;

async function fitsPage(page: Page) {
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(await page.evaluate(() => innerWidth));
}

async function record(page: Page, name: string) {
  await test
    .info()
    .attach(name, { body: await page.screenshot(), contentType: "image/png" });
}

for (const theme of themes) {
  for (const width of widths) {
    test(`composer ${width}px ${theme}: controls stay reachable focused and blurred`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/render-surfaces.html?view=combined&theme=${theme}&busy`,
      );
      const input = page.getByTestId("chat-form-textarea");
      await expect(input).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Stop generation", exact: true }),
      ).toBeVisible();
      await fitsPage(page);
      await record(page, "collapsed-composer");
      expect(
        await page
          .getByTestId("composer-toolbar")
          .evaluate((element) => element.getBoundingClientRect().height),
      ).toBeLessThanOrEqual(46);
      await input.focus();
      const more = page.getByRole("button", {
        name: "More composer controls",
        exact: true,
      });
      await more.click();
      await expect(page.getByTestId("worktree-control-trigger")).toBeVisible();
      await expect(
        page.getByRole("button", { name: /Agent.*86 tools/ }),
      ).toBeVisible();
      await expect(
        page.locator('[data-composer-control="model"] button').first(),
      ).toBeVisible();
      await fitsPage(page);
      await record(page, "overflow-composer");
      expect(
        await page
          .getByTestId("composer-toolbar")
          .evaluate((element) => element.getBoundingClientRect().height),
      ).toBeLessThanOrEqual(46);
      await page.keyboard.press("Escape");
      await input.fill(
        "First line of an unsent draft.\nSecond line with the complete acceptance criteria.\nThird line must remain available after blur.",
      );
      await page
        .getByRole("button", { name: "Outside composer", exact: true })
        .click();
      await expect(input).toHaveValue(/Third line must remain available/);
      expect(
        await input.evaluate((element) => element.clientHeight),
      ).toBeGreaterThan(70);
      await fitsPage(page);
      await record(page, "populated-composer-blurred");
    });

    test(`recovery ${width}px ${theme}: diagnostics wrap and histories stay bounded`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/render-surfaces.html?view=recovery&theme=${theme}&state=waiting&history`,
      );
      await page
        .getByText("Details and attempt history (100)", { exact: true })
        .click();
      const history = page.getByRole("region", {
        name: "Recovery diagnostics",
        exact: true,
      });
      await expect(history).toBeVisible();
      const geometry = await history.evaluate((element) => ({
        width: element.clientWidth,
        scrollWidth: element.scrollWidth,
        height: element.clientHeight,
        scrollHeight: element.scrollHeight,
      }));
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.width + 1);
      expect(geometry.height).toBeLessThanOrEqual(360);
      expect(geometry.scrollHeight).toBeGreaterThan(geometry.height);
      await expect(
        history.getByText("Original request: Failed", { exact: true }),
      ).toBeAttached();
      await expect(
        page.getByRole("button", { name: "Stop", exact: true }),
      ).toBeInViewport();
      await fitsPage(page);
      await record(page, "bounded-recovery");
    });

    test(`goals ${width}px ${theme}: compact status and large detail remain usable`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/render-surfaces.html?view=goal&theme=${theme}`,
      );
      await expect(
        page.getByRole("button", { name: "Pause goal", exact: true }),
      ).toBeVisible();
      await expect(page.getByText("Active", { exact: true })).toBeVisible();
      await expect(
        page.getByLabel("3 of 5 tasks completed", { exact: true }),
      ).toBeVisible();
      const status = await page
        .getByText("Active", { exact: true })
        .boundingBox();
      expect(status?.width).toBeGreaterThan(32);
      await fitsPage(page);
      await record(page, "compact-goal");
      await page.getByRole("button", { name: /^Goal set Active/ }).click();
      await page
        .getByRole("button", { name: "Goal Active", exact: true })
        .click();
      await expect(
        page.getByRole("button", { name: "Pause", exact: true }),
      ).toBeInViewport();
      await expect(
        page.getByRole("button", { name: "Stop", exact: true }),
      ).toBeInViewport();
      await record(page, "expanded-goal");
      await page
        .getByRole("button", { name: "Open full goal and plan", exact: true })
        .click();
      const dialog = page.getByRole("dialog", {
        name: "Full goal and plan",
        exact: true,
      });
      await expect(dialog).toBeVisible();
      await expect(
        dialog.getByRole("heading", {
          name: "Approved execution plan",
          exact: true,
        }),
      ).toBeAttached();
      await fitsPage(page);
      await record(page, "full-goal-reader");
      await page.keyboard.press("Escape");
      await expect(dialog).toBeHidden();
      await expect(
        page.getByRole("button", {
          name: "Open full goal and plan",
          exact: true,
        }),
      ).toBeFocused();
    });
  }
}

for (const virtual of [false, true]) {
  test(`Git ${
    virtual ? "virtual" : "ordinary"
  }: compact rows and keyboard selection at 320px`, async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 900 });
    await page.goto(
      `/tests/e2e/render-surfaces.html?view=git${virtual ? "&virtual" : ""}`,
    );
    const tree = page.getByRole("tree", { name: "Changes", exact: true });
    await expect(tree).toBeVisible();
    await expect(tree.getByRole("treeitem").first()).toBeVisible();
    const pitch = await tree
      .getByRole("treeitem")
      .first()
      .evaluate((element) => ({
        row: element.getBoundingClientRect().height,
        wrapper: element.parentElement?.getBoundingClientRect().height,
      }));
    expect(pitch.row).toBe(26);
    if (virtual) expect(pitch.wrapper).toBe(26);
    await tree.focus();
    await page.keyboard.press("End");
    await expect
      .poll(() =>
        tree.evaluate((element) =>
          Boolean(
            document.getElementById(
              element.getAttribute("aria-activedescendant") ?? "",
            ),
          ),
        ),
      )
      .toBe(true);
    const lastIndex = virtual ? 93 : 9;
    const lastFile = `railgeom/branch_9/deep_${lastIndex}/observations_${"long_name_".repeat(
      12,
    )}_${lastIndex}.py`;
    const selected = tree.getByRole("treeitem", {
      name: lastFile,
      exact: true,
    });
    await expect(selected).toHaveAttribute("aria-selected", "true");
    const include = selected.getByRole("checkbox");
    await expect(include).toBeChecked();
    await page.keyboard.press("Space");
    await expect(include).not.toBeChecked();
    await fitsPage(page);
    await record(page, "git-end-selection");
  });
}

test("large Git count is compact with an exact accessible description", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=git&huge");
  await expect(
    page.getByRole("button", { name: "Git", exact: true }),
  ).toHaveAccessibleDescription("10000 changed files");
  await expect(page.getByText("99+", { exact: true })).toBeVisible();
  await fitsPage(page);
});

test("large goal editor preserves one shared draft across expansion", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=setup&expanded");
  const draft = page.getByRole("textbox", { name: "Goal text", exact: true });
  await draft.fill("Preserve the complete original goal and all constraints.");
  await page
    .getByRole("button", { name: "Expand editor", exact: true })
    .click();
  await expect(
    page.getByRole("dialog", { name: "Set a goal", exact: true }),
  ).toBeVisible();
  await expect(draft).toHaveCount(1);
  await draft.fill("Expanded draft with every original constraint preserved.");
  await record(page, "expanded-goal-editor");
  await page
    .getByRole("button", { name: "Return to goal", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Expand editor", exact: true }),
  ).toBeFocused();
  await expect(draft).toHaveValue(
    "Expanded draft with every original constraint preserved.",
  );
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toBeEnabled();
  await fitsPage(page);
});

test("More controls support actual actions and nested selector focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page.getByTestId("chat-form-textarea").focus();
  const more = page.getByRole("button", {
    name: "More composer controls",
    exact: true,
  });
  await more.click();
  const approve = page.getByRole("button", {
    name: "Auto-approve file editing tools",
    exact: true,
  });
  await expect(approve).toHaveAttribute("aria-pressed", "false");
  await approve.click();
  await expect(more).toHaveAttribute("aria-expanded", "true");
  await expect(approve).toHaveAttribute("aria-pressed", "true");
  const mode = page.locator('[data-composer-control="mode"] button').first();
  await mode.click();
  await expect(page.getByText("Discuss code", { exact: true })).toBeVisible();
  await fitsPage(page);
  await record(page, "nested-mode-selector");
  await page.keyboard.press("Escape");
  await expect(more).toHaveAttribute("aria-expanded", "true");
  await expect(mode).toBeFocused();
  await page.getByTestId("worktree-control-trigger").click();
  await expect(
    page.getByText("Worktrees", { exact: true }).first(),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(more).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Escape");
  await expect(more).toBeFocused();
});

test("recovered and failed recovery states preserve labels and original model choice", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto(
    "/tests/e2e/render-surfaces.html?view=recovery&state=succeeded",
  );
  await page
    .getByText("Recovered after 1 new request", { exact: true })
    .click();
  await expect(
    page.getByText("Original request: Failed", { exact: true }),
  ).toBeVisible();
  await fitsPage(page);
  await page.goto("/tests/e2e/render-surfaces.html?view=recovery");
  await page
    .getByRole("button", { name: "Retry options", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Select model", exact: true }),
  ).toHaveText("Keep original model");
  await fitsPage(page);
  await record(page, "recovery-options");
});

test("resizing an open scope selector retains its anchor and returns focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page.getByTestId("chat-form-textarea").focus();
  const mode = page.locator('[data-composer-control="mode"] button').first();
  await mode.click();
  await expect(page.getByText("Discuss code", { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 640, height: 900 });
  await expect(mode).toBeAttached();
  await expect(mode).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(mode).toBeFocused();
  await page.setViewportSize({ width: 320, height: 900 });
  const more = page.getByRole("button", {
    name: "More composer controls",
    exact: true,
  });
  await expect(more).toHaveAttribute("aria-expanded", "true");
  await expect(mode).toBeVisible();
  await fitsPage(page);
});

test("resizing a later focused control opens More without stealing focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1500, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  const worktree = page.getByTestId("worktree-control-trigger");
  await expect(worktree).toBeVisible();
  await worktree.focus();
  await page.setViewportSize({ width: 320, height: 900 });
  await expect(
    page.getByRole("button", { name: "More composer controls", exact: true }),
  ).toHaveAttribute("aria-expanded", "true");
  await expect(worktree).toBeFocused();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await expect(worktree).toBeFocused();
});

test("composer respects reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page.getByTestId("chat-form-textarea").focus();
  expect(
    await page
      .locator('[class*="inputHeader"]')
      .evaluate((element) => getComputedStyle(element).animationName),
  ).toBe("none");
});
