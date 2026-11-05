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
        if (
          child.getAttribute("data-hidden") === "true" ||
          child.getAttribute("data-empty") === "true"
        )
          return false;
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
      const form = input.locator("xpath=ancestor::form");
      await expect(form).toHaveClass(/chatFormCollapsed/);
      await expect(
        page.getByRole("button", { name: /^(Expand|Collapse) composer$/ }),
      ).toHaveCount(0);
      await input.click();
      await expect(form).toHaveClass(/chatFormExpanded/);
      await expect(page.getByTestId("composer-top-toolbar")).toBeVisible();
      await expect
        .poll(async () => {
          const top = await page
            .getByTestId("composer-top-toolbar")
            .boundingBox();
          const editor = await input.boundingBox();
          return top!.y + top!.height - editor!.y;
        })
        .toBeLessThanOrEqual(1);
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
      await expect(form).toHaveClass(/chatFormExpanded/);
      await record(page, "populated-composer-blurred");
      await input.fill("");
      await page
        .getByRole("button", { name: "Outside composer", exact: true })
        .click();
      await expect(form).toHaveClass(/chatFormCollapsed/);
      await page
        .getByRole("button", { name: "Stop generation", exact: true })
        .click();
      await expect(form).toHaveClass(/chatFormCollapsed/);
    });

    test(`recovery ${width}px ${theme}: diagnostics wrap and failed history remains readable`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/render-surfaces.html?view=recovery&theme=${theme}&state=waiting&history`,
      );
      await page.getByRole("button", { name: "Details", exact: true }).click();
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
      expect(geometry.scrollHeight).toBeGreaterThan(0);
      await expect(
        history.getByText("Original request: Failed", { exact: true }),
      ).toBeAttached();
      await page
        .getByRole("button", { name: "Stop", exact: true })
        .scrollIntoViewIfNeeded();
      await expect(
        page.getByRole("button", { name: "Stop", exact: true }),
      ).toBeInViewport();
      await fitsPage(page);
      await record(page, "bounded-recovery");
      if (width === 240) {
        await test.info().attach("recovery-stop-geometry", {
          body: JSON.stringify({
            theme,
            viewport: page.viewportSize(),
            stop: await page
              .getByRole("button", { name: "Stop", exact: true })
              .boundingBox(),
          }),
          contentType: "application/json",
        });
      }
      const raw = `Connection reset\nhttps://provider.example.com/${"unbroken_provider_segment_".repeat(
        40,
      )}\nLast diagnostic line`;
      await expect(page.getByRole("alert")).toHaveText(raw);
      await expect(history.getByText(/Retry 99: Failed/)).toBeAttached();
    });

    test(`goals ${width}px ${theme}: compact status and large detail remain usable`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(
        `/tests/e2e/render-surfaces.html?view=goal&theme=${theme}`,
      );
      await expect(page.getByTestId("transcript-verb")).toHaveText("Goal");
      await expect(page.getByText("Active", { exact: true })).toBeVisible();
      await expect(
        page.getByLabel("3 of 5 tasks completed", { exact: true }),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Pause goal", exact: true }),
      ).toHaveCount(0);
      await fitsPage(page);
      await record(page, "compact-goal");
      await page.getByTestId("transcript-toggle").click();
      await expect(
        page.getByRole("button", { name: "Pause goal", exact: true }),
      ).toBeInViewport();
      await expect(
        page.getByRole("button", { name: "Stop goal", exact: true }),
      ).toBeInViewport();
      await record(page, "expanded-goal");
      const details = page.getByRole("button", {
        name: "Goal details",
        exact: true,
      });
      await details.click();
      const dialog = page.getByRole("dialog", {
        name: "Goal details",
        exact: true,
      });
      await expect(dialog).toBeVisible();
      await expect(
        dialog.getByRole("region", { name: "Goal plan", exact: true }),
      ).toBeAttached();
      await fitsPage(page);
      await record(page, "full-goal-reader");
      await page.keyboard.press("Escape");
      await expect(dialog).toBeHidden();
      await expect(details).toBeFocused();
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

test("goal editor dialog opens from the expanded goal row and returns focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=setup&expanded");
  const create = page.getByRole("button", { name: "Create goal", exact: true });
  await create.click();
  const dialog = page.getByRole("dialog", { name: "Set a goal", exact: true });
  await expect(dialog).toBeVisible();
  const draft = dialog.getByRole("textbox", { name: "Goal text", exact: true });
  await draft.fill("Preserve the complete original goal and all constraints.");
  await expect(
    dialog.getByRole("button", { name: "Set goal", exact: true }),
  ).toBeEnabled();
  await record(page, "expanded-goal-editor");
  await fitsPage(page);
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(create).toBeFocused();
});

test("individual permissions and nested selector retain state without a More menu", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page.getByTestId("chat-form-textarea").click();
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
    .getByRole("button", { name: "Outside composer", exact: true })
    .click();
  await expect(page.locator("form")).toHaveClass(/chatFormCollapsed/);
  await page.getByTestId("chat-form-textarea").click();
  await expect(approve).toHaveAttribute("aria-pressed", "true");
});

test("recovered and failed recovery states preserve labels and original model choice", async ({
  page,
}) => {
  await page.setViewportSize({ width: 320, height: 900 });
  await page.goto(
    "/tests/e2e/render-surfaces.html?view=recovery&state=succeeded",
  );
  await expect(
    page.getByRole("region", { name: "Recovery status", exact: true }),
  ).toHaveCount(0);
  await fitsPage(page);
  await page.goto("/tests/e2e/render-surfaces.html?view=recovery");
  await page.getByRole("button", { name: "Options", exact: true }).click();
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
  await page.getByTestId("chat-form-textarea").click();
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
  await page.emulateMedia({ reducedMotion: "reduce" });
  test.setTimeout(90_000);
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=composer");
  await page.getByTestId("chat-form-textarea").click();
  const row = page.getByTestId("composer-toolbar");
  const policy = [
    "threadInfo",
    "sidebar",
    "wand",
    "autoCompact",
    "enrichment",
    "browser",
    "mic",
    "image",
    "skills",
    "worktree",
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
    if (hidden.length > 0) {
      await expect(
        page.locator('[data-composer-control="model"]'),
      ).toHaveAttribute("data-level", "2");
      await expect(
        page.locator('[data-composer-control="worktree"]'),
      ).toHaveAttribute("data-level", "1");
      await expect(
        page.locator('[data-composer-control="mode"]'),
      ).toHaveAttribute("data-level", /[12]/);
    }
    await expect(
      page.locator('[data-composer-control="model"]'),
    ).toHaveAttribute("data-hidden", "false");
    await expect(
      page.locator('[data-composer-control="send"]'),
    ).toHaveAttribute("data-hidden", "false");
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

test("context warnings keep usage visible with a busy narrow draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 240, height: 900 });
  await page.goto(
    "/tests/e2e/render-surfaces.html?view=composer&context-warning&busy&paid",
  );
  await page.getByTestId("chat-form-textarea").fill("Keep this unsent draft.");
  await expect(
    page.locator('[data-composer-control="context"]'),
  ).toHaveAttribute("data-hidden", "false");
  await expect(
    page.locator('[data-composer-control="context"] [class*="isWarning"]'),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Context usage details", exact: true }),
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
  await input.evaluate((element: HTMLTextAreaElement) => {
    element.dataset.identity = "original";
  });
  await input.click();
  await expect(page.locator("form")).toHaveClass(/chatFormExpanded/);
  await page
    .getByRole("button", { name: "Outside composer", exact: true })
    .click();
  await expect(page.locator("form")).toHaveClass(/chatFormCollapsed/);
  await input.focus();
  await expect(page.locator("form")).toHaveClass(/chatFormExpanded/);
  await input.fill("first line\nsecond line\nthird line");
  await input.evaluate((element: HTMLTextAreaElement) => {
    element.setSelectionRange(3, 8);
  });
  await page
    .getByRole("button", { name: "Outside composer", exact: true })
    .click();
  await expect(page.locator("form")).toHaveClass(/chatFormExpanded/);
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
  await page.getByTestId("chat-form-textarea").click();
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

test("streaming composer keeps the dock top block clickable while expanded", async ({
  page,
}) => {
  await page.setViewportSize({ width: 768, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=combined&busy");
  const input = page.getByTestId("chat-form-textarea");
  const form = input.locator("xpath=ancestor::form");
  await input.click();
  await expect(form).toHaveClass(/chatFormExpanded/);
  const toggle = page.getByTestId("transcript-toggle");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(form).toHaveClass(/chatFormExpanded/);
  const details = page.getByRole("button", {
    name: "Goal details",
    exact: true,
  });
  await details.click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByText("Close", { exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(details).toBeFocused();
  await expect(form).toHaveClass(/chatFormExpanded/);
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(form).toHaveClass(/chatFormExpanded/);
  await page
    .getByRole("button", { name: "Outside composer", exact: true })
    .click();
  await expect(form).toHaveClass(/chatFormCollapsed/);
});

for (const expanded of [false, true]) {
  test(`usage rings open on hover without changing a ${
    expanded ? "expanded" : "collapsed"
  } streaming composer`, async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/tests/e2e/render-surfaces.html?view=combined&busy");
    const input = page.getByTestId("chat-form-textarea");
    const form = input.locator("xpath=ancestor::form");
    if (expanded) {
      await input.click();
      await expect(form).toHaveClass(/chatFormExpanded/);
    } else {
      await expect(form).toHaveClass(/chatFormCollapsed/);
    }
    const state = expanded ? /chatFormExpanded/ : /chatFormCollapsed/;
    const rings = [
      {
        trigger: page.getByRole("button", {
          name: "Context usage details",
          exact: true,
        }),
        content: page.getByText("Context usage", { exact: true }),
      },
      {
        trigger: page.getByRole("button", { name: /: 42% used$/ }),
        content: page.getByText("Daily usage", { exact: true }),
      },
    ];
    for (const ring of rings) {
      await ring.trigger.hover();
      await expect(ring.content).toBeVisible();
      await expect(form).toHaveClass(state);
      await page.waitForTimeout(400);
      await expect(ring.content).toBeVisible();
      await expect(form).toHaveClass(state);
      await page.mouse.move(5, 5);
      await expect(ring.content).toBeHidden();
      await expect(form).toHaveClass(state);
    }
    if (expanded) await expect(input).toBeFocused();
  });
}

test("hovered context popover stays open on click and its tabs work", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/tests/e2e/render-surfaces.html?view=combined&busy");
  const form = page
    .getByTestId("chat-form-textarea")
    .locator("xpath=ancestor::form");
  const trigger = page.getByRole("button", {
    name: "Context usage details",
    exact: true,
  });
  await trigger.hover();
  const heading = page.getByText("Context usage", { exact: true });
  await expect(heading).toBeVisible();
  await trigger.click();
  await page.waitForTimeout(300);
  await expect(heading).toBeVisible();
  await page.getByRole("tab", { name: "Breakdown" }).click();
  await expect(page.getByRole("tab", { name: "Breakdown" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(heading).toBeVisible();
  await expect(form).toHaveClass(/chatFormCollapsed/);
  await page.keyboard.press("Escape");
  await expect(heading).toBeHidden();
});
