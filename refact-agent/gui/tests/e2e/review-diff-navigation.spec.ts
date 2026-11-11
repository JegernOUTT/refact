import { expect, test } from "@playwright/test";

test("Review diff fills its pane and exposes keyboard hunk and comment navigation", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html");
  await page.getByRole("radio", { name: /^Changes/ }).press("Space");
  const host = page
    .getByTestId("review-diff-editors")
    .filter({ visible: true });
  await expect(host).toBeVisible();
  await expect(
    page.getByRole("list", { name: "Changed files" }).getByRole("button"),
  ).toHaveCount(1);
  await expect(
    page.getByRole("group", { name: "Diff hunks" }).getByRole("status"),
  ).toHaveText("1 / 2 changes");
  await page
    .getByRole("button", { name: "Next change", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("group", { name: "Diff hunks" }).getByRole("status"),
  ).toHaveText("2 / 2 changes");
  await page
    .getByRole("button", { name: "Previous change", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("group", { name: "Diff hunks" }).getByRole("status"),
  ).toHaveText("1 / 2 changes");
  const geometry = await host.evaluate((node) => {
    const pane = node.parentElement!.parentElement!;
    return {
      bottom: node.getBoundingClientRect().bottom,
      paneBottom: pane.getBoundingClientRect().bottom,
      height: node.getBoundingClientRect().height,
      paneHeight: pane.getBoundingClientRect().height,
    };
  });
  expect(geometry.height).toBeGreaterThan(geometry.paneHeight * 0.65);
  expect(Math.abs(geometry.bottom - geometry.paneBottom)).toBeLessThan(2);
  await page
    .getByRole("button", { name: "Next comment", exact: true })
    .press("Enter");
  await expect(
    page.locator('[data-review-thread-id="thread-1"][data-active="true"]'),
  ).toBeVisible();
  await page.evaluate(() => window.__reviewFixture?.publishTopic());
  await expect(page.getByRole("radio", { name: /^Changes/ })).toBeChecked();
});

test("unchanged files remain separate from changed files when shown and selected", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html");
  await page.getByRole("radio", { name: /^Changes/ }).press("Space");
  const changed = page.getByRole("list", {
    name: "Changed files",
    exact: true,
  });
  const unchanged = page.getByRole("list", {
    name: "Unchanged files",
    exact: true,
  });
  await expect(changed.getByRole("button")).toHaveCount(1);
  await expect(unchanged).toHaveCount(0);
  await page.getByRole("button", { name: "More review options" }).click();
  await page.getByRole("menuitem", { name: "Show unchanged files" }).click();
  await expect(changed.getByRole("button")).toHaveCount(1);
  await expect(changed.getByRole("button")).toHaveAttribute(
    "aria-label",
    "src/parse.ts",
  );
  await expect(unchanged.getByRole("button")).toHaveCount(1);
  const readme = unchanged.getByRole("button", {
    name: "README.md",
    exact: true,
  });
  await readme.click();
  await expect(readme).toHaveAttribute("aria-current", "true");
  await page.getByRole("button", { name: "More review options" }).click();
  await page.getByRole("menuitem", { name: "Hide unchanged files" }).click();
  await expect(unchanged).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Unchanged files" }),
  ).toHaveCount(0);
  await expect(changed.getByRole("button")).toHaveCount(1);
  await expect(changed.getByRole("button")).toHaveAttribute(
    "aria-current",
    "true",
  );
  await expect(page.locator('[class*="_filePath_"]')).toHaveText(
    "src/parse.ts",
  );
  await expect(
    page.getByTestId("review-diff-editors").filter({ visible: true }),
  ).toBeVisible();
});

test("long files restore scroll and traverse cross-file and historical comments", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?diffNavigation=1");
  await page.getByRole("radio", { name: /^Changes/ }).press("Space");
  await page.getByRole("radio", { name: "Split" }).locator("..").click();
  const files = page.getByRole("list", { name: "Changed files" });
  await expect(files.getByRole("button")).toHaveCount(2);
  const host = page
    .getByTestId("review-diff-editors")
    .filter({ visible: true });
  const rightScroll = () => host.locator(".cm-scroller").last();

  await expect
    .poll(() => rightScroll().evaluate((node) => node.scrollTop))
    .toBeGreaterThan(800);
  await files
    .getByRole("button", { name: "src/parse.ts", exact: true })
    .click();
  await expect
    .poll(() => rightScroll().evaluate((node) => node.scrollTop))
    .toBeGreaterThan(800);
  const beforeWheel = await rightScroll().evaluate((node) => node.scrollTop);
  await rightScroll().hover();
  await page.mouse.wheel(0, -700);
  await expect
    .poll(() => rightScroll().evaluate((node) => node.scrollTop))
    .toBeLessThan(beforeWheel);
  const savedTop = await rightScroll().evaluate(
    (node) =>
      new Promise<number>((resolve) => {
        let previous = node.scrollTop;
        let stableFrames = 0;
        const measure = () => {
          const top = node.scrollTop;
          stableFrames = top === previous ? stableFrames + 1 : 0;
          previous = top;
          if (stableFrames >= 5) resolve(top);
          else requestAnimationFrame(measure);
        };
        requestAnimationFrame(measure);
      }),
  );
  await files.getByRole("button", { name: "README.md", exact: true }).click();
  await files
    .getByRole("button", { name: "src/parse.ts", exact: true })
    .click();
  await expect
    .poll(() => rightScroll().evaluate((node) => node.scrollTop))
    .toBe(savedTop);
  await page.evaluate(() => window.__reviewFixture?.publishTopic());
  await expect
    .poll(() => rightScroll().evaluate((node) => node.scrollTop))
    .toBe(savedTop);
  const settledTops = await host.locator(".cm-scroller").evaluateAll(
    (nodes) =>
      new Promise<number[]>((resolve) => {
        let frames = 5;
        const measure = () => {
          if (--frames === 0) resolve(nodes.map((node) => node.scrollTop));
          else requestAnimationFrame(measure);
        };
        requestAnimationFrame(measure);
      }),
  );
  expect(settledTops).toEqual([savedTop, savedTop]);
  await page
    .getByRole("button", { name: "Next change", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("group", { name: "Diff hunks" }).getByRole("status"),
  ).toHaveText("2 / 2 changes");
  await page
    .getByRole("button", { name: "Next comment", exact: true })
    .press("Enter");
  await expect(
    files.getByRole("button", { name: "README.md", exact: true }),
  ).toHaveAttribute("aria-current", "true");
  await expect(
    page.locator(
      '[data-review-thread-id="readme-comment"][data-active="true"]',
    ),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Next comment", exact: true })
    .press("Enter");
  await expect(
    page.getByRole("region", { name: "Historical comment" }),
  ).toContainText("without guessing a line");
  await expect(
    page.locator(
      '[data-review-thread-id="stalled-comment"][data-active="true"]',
    ),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Next comment", exact: true })
    .press("Enter");
  await expect(
    files.getByRole("button", { name: "src/parse.ts", exact: true }),
  ).toHaveAttribute("aria-current", "true");
  await expect(
    page.locator('[data-review-thread-id="thread-1"][data-active="true"]'),
  ).toBeVisible();
});

test("shows an empty unchanged-files item when every file changed", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?diffNavigation=1");
  await page.getByRole("radio", { name: /^Changes/ }).press("Space");
  await page.getByRole("button", { name: "More review options" }).click();
  await page.getByRole("menuitem", { name: "Show unchanged files" }).click();
  const unchanged = page.getByRole("list", {
    name: "Unchanged files",
    exact: true,
  });
  await expect(unchanged.getByRole("listitem")).toHaveText(
    "No unchanged files.",
  );
  await expect(unchanged.getByRole("button")).toHaveCount(0);
});

test("split panes share wheel, hunk and discussion scrolling", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?diffNavigation=1");
  await page.getByRole("radio", { name: /^Changes/ }).press("Space");
  await page.getByRole("radio", { name: "Split" }).locator("..").click();
  const host = page
    .getByTestId("review-diff-editors")
    .filter({ visible: true });
  const scrollers = host.locator(".cm-scroller");
  await expect(scrollers).toHaveCount(2);
  const tops = () =>
    scrollers.evaluateAll((nodes) => nodes.map((node) => node.scrollTop));
  const aligned = async (label: string) => {
    await expect
      .poll(async () => {
        const [left, right] = await tops();
        return left === right && right > 0;
      })
      .toBe(true);
    console.log(`${label}: ${JSON.stringify(await tops())}`);
  };
  await aligned("initial");
  const beforeRight = await tops();
  await scrollers.last().hover();
  await page.mouse.wheel(0, -400);
  await expect.poll(async () => (await tops())[1]).toBeLessThan(beforeRight[1]);
  await aligned("right wheel");
  const beforeLeft = await tops();
  await scrollers.first().hover();
  await page.mouse.wheel(0, -400);
  await expect.poll(async () => (await tops())[0]).toBeLessThan(beforeLeft[0]);
  await aligned("left wheel");
  await page.getByRole("button", { name: "Next change", exact: true }).click();
  await expect(
    page.getByRole("group", { name: "Diff hunks" }).getByRole("status"),
  ).toHaveText("2 / 2 changes");
  await aligned("next change");
  await page
    .getByRole("button", { name: "Previous change", exact: true })
    .click();
  await expect(
    page.getByRole("group", { name: "Diff hunks" }).getByRole("status"),
  ).toHaveText("1 / 2 changes");
  await aligned("previous change");
  await page.getByRole("button", { name: "Next comment", exact: true }).click();
  await expect(
    page.locator(
      '[data-review-thread-id="readme-comment"][data-active="true"]',
    ),
  ).toBeVisible();
  await expect
    .poll(async () => {
      const [left, right] = await tops();
      return left === right;
    })
    .toBe(true);
  console.log(`discussion: ${JSON.stringify(await tops())}`);
});
