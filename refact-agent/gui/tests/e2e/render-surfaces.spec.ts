import { expect, test, type Page } from "@playwright/test";

const widths = [240, 320, 360, 480, 640, 768, 1280];
const themes = ["light", "dark"] as const;

async function fitsPage(page: Page) {
  expect(
    await page.evaluate(() => document.documentElement.scrollWidth),
  ).toBeLessThanOrEqual(await page.evaluate(() => innerWidth));
}

async function fitsToolbar(page: Page) {
  const overflow = await page
    .getByTestId("composer-toolbar")
    .evaluate((row) => {
      const bounds = row.getBoundingClientRect();
      const padding = parseFloat(getComputedStyle(row).paddingRight);
      return Array.from(row.children).some((child) => {
        if (child.getAttribute("data-hidden") === "true") return false;
        const rect = child.getBoundingClientRect();
        return (
          rect.left < bounds.left || rect.right > bounds.right - padding + 1
        );
      });
    });
  expect(overflow).toBe(false);
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
      const expand = page.getByRole("button", {
        name: "Expand composer",
        exact: true,
      });
      await expect(expand).toHaveAttribute("aria-expanded", "false");
      await expand.click();
      const collapse = page.getByRole("button", {
        name: "Collapse composer",
        exact: true,
      });
      await expect(collapse).toHaveAttribute("aria-expanded", "true");
      await expect(
        page.getByRole("button", { name: "More composer controls" }),
      ).toHaveCount(0);
      await fitsPage(page);
      await record(page, "expanded-composer");
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

test("individual permissions and nested selector retain state without a More menu", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page
    .getByRole("button", { name: "Expand composer", exact: true })
    .click();
  const approve = page.getByRole("button", {
    name: "Auto-approve file editing tools",
    exact: true,
  });
  await expect(approve).toHaveAttribute("aria-pressed", "false");
  await approve.click();
  await expect(approve).toHaveAttribute("aria-pressed", "true");
  const mode = page.locator('[data-composer-control="mode"] button').first();
  await mode.click();
  await expect(page.getByText("Discuss code", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(mode).toBeFocused();
  await page
    .getByRole("button", { name: "Collapse composer", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Expand composer", exact: true })
    .click();
  await expect(approve).toHaveAttribute("aria-pressed", "true");
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

test("resizing an open selector preserves its trigger and keeps Stop inside the toolbar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer&busy");
  await page
    .getByRole("button", { name: "Expand composer", exact: true })
    .click();
  const context = page.locator('[data-composer-control="context"] button');
  await context.click();
  await expect(context).toHaveAttribute("aria-expanded", "true");
  await page.setViewportSize({ width: 240, height: 900 });
  await expect(context).toBeVisible();
  await expect(
    page.locator('button[aria-label="Stop generation"]'),
  ).toBeInViewport();
  await fitsToolbar(page);
  await page.keyboard.press("Escape");
  await expect(
    page.locator('[data-composer-control="context"]'),
  ).toHaveAttribute("data-hidden", "false");
  await expect(context).toBeFocused();
  await fitsPage(page);
});

test("real control widths hide rare actions individually and restore their order", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page
    .getByRole("button", { name: "Expand composer", exact: true })
    .click();
  const row = page.getByTestId("composer-toolbar");
  const policy = [
    "threadInfo",
    "sidebar",
    "wand",
    "browser",
    "enrichment",
    "autoCompact",
    "mic",
    "image",
    "worktree",
    "model",
    "mode",
  ];
  const candidates = await row.evaluate(
    (element, priority) =>
      priority.filter((id) => {
        const host = element.querySelector(`[data-composer-control="${id}"]`);
        return host && host.getBoundingClientRect().width > 0;
      }),
    policy,
  );
  const visibleAt = async (width: number) => {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    await fitsToolbar(page);
    return row.evaluate(
      (element, ids) =>
        ids.filter(
          (id) =>
            element
              .querySelector(`[data-composer-control="${id}"]`)
              ?.getAttribute("data-hidden") === "true",
        ),
      candidates,
    );
  };
  let previous: string[] = [];
  const transitions: Array<{ width: number; hidden: string[] }> = [];
  for (let width = 1000; width >= 240; width -= 4) {
    const hidden = await visibleAt(width);
    expect(hidden).toEqual(candidates.slice(0, hidden.length));
    if (hidden.length !== previous.length) {
      expect(hidden.length - previous.length).toBe(1);
      transitions.push({ width, hidden });
    }
    previous = hidden;
  }
  expect(transitions.length).toBeGreaterThan(5);
  for (const { width, hidden } of transitions.reverse())
    expect(await visibleAt(width)).toEqual(hidden);
  expect(await visibleAt(1600)).toEqual([]);
});

test("context warnings keep usage and compression visible with a busy compact draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 240, height: 900 });
  await page.goto(
    "/tests/e2e/render-surfaces.html?view=composer&context-warning&busy&paid",
  );
  await page.getByTestId("chat-form-textarea").fill("Keep this unsent draft.");
  await expect(
    page.getByText("Context usage is high", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Context usage details", exact: true }),
  ).toBeInViewport();
  await expect(
    page.getByRole("button", {
      name: "Compress or rebuild context",
      exact: true,
    }),
  ).toBeInViewport();
  await expect(
    page.getByRole("button", { name: "Stop generation", exact: true }),
  ).toBeInViewport();
  await fitsToolbar(page);
  const geometry = await page
    .locator('[data-composer-control="context"]')
    .evaluate((host) => {
      const trigger = host.querySelector("button");
      if (!trigger) return false;
      const outer = host.getBoundingClientRect();
      const inner = trigger.getBoundingClientRect();
      return (
        inner.width >= 28 &&
        inner.left >= outer.left &&
        inner.right <= outer.right + 1
      );
    });
  expect(geometry).toBe(true);
});

test("collapse and expansion preserve the same editor, selection and draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  const input = page.getByTestId("chat-form-textarea");
  await input.fill("first line\nsecond line\nthird line");
  await input.evaluate((element: HTMLTextAreaElement) => {
    element.dataset.identity = "original";
    element.setSelectionRange(3, 8);
  });
  await page
    .getByRole("button", { name: "Expand composer", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Collapse composer", exact: true })
    .click();
  await expect(input).toHaveAttribute("data-identity", "original");
  await expect(input).toHaveValue("first line\nsecond line\nthird line");
  expect(
    await input.evaluate((element: HTMLTextAreaElement) => [
      element.selectionStart,
      element.selectionEnd,
    ]),
  ).toEqual([3, 8]);
  await fitsPage(page);
});

test("an empty compact top row restores permissions after expansion and widening", async ({
  page,
}) => {
  await page.setViewportSize({ width: 240, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page
    .getByRole("button", { name: "Expand composer", exact: true })
    .click();
  await page.setViewportSize({ width: 1600, height: 900 });
  await expect(
    page.getByRole("button", {
      name: "Configure project information",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Auto-approve file editing tools",
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Auto-approve dangerous commands",
      exact: true,
    }),
  ).toBeVisible();
  await fitsPage(page);
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
