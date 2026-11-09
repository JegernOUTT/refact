import { expect, test } from "@playwright/test";

for (const { width, height, theme, motion } of [
  { width: 1440, height: 900, theme: "light", motion: "no-preference" },
  { width: 768, height: 800, theme: "dark", motion: "reduce" },
  { width: 390, height: 420, theme: "light", motion: "reduce" },
] as const) {
  test(`review controls remain operable ${width}x${height} ${theme} ${motion}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ colorScheme: theme, reducedMotion: motion });
    await page.goto(`/tests/e2e/review-canvas.html?tour=rich&theme=${theme}`);
    if (width <= 680) {
      await page
        .getByRole("radio", { name: "Canvas", exact: true })
        .locator("..")
        .click();
    }
    await expect(
      page.getByRole("heading", { name: "Input trimming" }),
    ).toBeVisible();
    await page.getByText("Review", { exact: true }).first().click();
    await expect(
      page.getByRole("radio", { name: "Review", exact: true }),
    ).toBeChecked();
    await page.getByRole("radio", { name: "Unified" }).locator("..").click();
    await expect(page.getByRole("radio", { name: "Unified" })).toBeChecked();
    await page.getByRole("radio", { name: "Split" }).locator("..").click();
    await expect(page.getByRole("radio", { name: "Split" })).toBeChecked();
    await page.getByRole("radio", { name: "Since last" }).locator("..").click();
    await expect(page.getByRole("radio", { name: "Since last" })).toBeChecked();
    await page.getByRole("radio", { name: "Full" }).locator("..").click();
    await expect(page.getByRole("radio", { name: "Full" })).toBeChecked();
    await page.getByRole("button", { name: "More review options" }).click();
    await page.getByRole("menuitem", { name: "Show unchanged files" }).click();
    if (width <= 768) {
      await page
        .getByRole("button", { name: "Browse files", exact: true })
        .click();
    }
    await expect(
      page.getByRole("button", { name: "README.md", exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: test.info().outputPath("review-layout.png"),
    });
    await page.getByRole("button", { name: "More review options" }).click();
    await page.getByRole("menuitem", { name: "Hide unchanged files" }).click();
  });
}

test("empty review activates without stealing composer focus", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?state=empty&tour=rich");
  const chat = page.getByRole("textbox", { name: "Chat composer" });
  await chat.fill("Draft survives activation");
  await chat.focus();
  await page.evaluate(() => window.__reviewFixture?.activate());
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await expect(chat).toBeFocused();
  await expect(chat).toHaveValue("Draft survives activation");
});

test("worker cancellation is requested in fixture", async ({ page }) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=rich");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Workers \(/ }).click();
  await page
    .getByRole("button", { name: "Cancel Probe empty-input behaviour" })
    .click();
  await expect(page.getByText("Stopping…")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Cancel Probe empty-input behaviour" }),
  ).toBeDisabled();
});

test("candidate selector returns from history and sources reveal a file", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=rich");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.getByRole("combobox", { name: "Candidate revision" }).click();
  await page.getByRole("option", { name: /^#1 / }).click();
  await expect(
    page.getByRole("button", { name: "Back to current" }),
  ).toBeVisible();
  await expect(page.getByRole("radio", { name: "Since last" })).toBeDisabled();
  await page.getByRole("button", { name: "Back to current" }).click();
  await expect(
    page.getByRole("button", { name: "Back to current" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "Sources & Evidence" }).click();
  await expect(
    page.getByRole("button", { name: "Sources & Evidence" }),
  ).toHaveAttribute("aria-expanded", "true");
  await page
    .getByRole("article", {
      name: "Evidence: total() implementation consulted",
    })
    .getByRole("button", { name: "Show range in diff", exact: true })
    .click();
  await page.screenshot({ path: test.info().outputPath("source-reveal.png") });
  await expect(
    page.getByRole("radio", { name: "Review", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("textbox", { name: "Head bbbbbbbb" }),
  ).toBeVisible();
});

test("short mobile viewport reaches file navigation and diff", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 420 });
  await page.goto("/tests/e2e/review-canvas.html?tour=rich");
  await page
    .getByRole("radio", { name: "Canvas", exact: true })
    .locator("..")
    .click();
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.getByText("Review", { exact: true }).first().click();
  await page.getByRole("button", { name: "Browse files", exact: true }).click();
  const file = page.getByRole("button", { name: "src/parse.ts" });
  await file.scrollIntoViewIfNeeded();
  await file.focus();
  await expect(file).toBeFocused();
  await page.keyboard.press("Enter");
  await page.screenshot({ path: test.info().outputPath("mobile-file.png") });
  const diff = page.getByRole("textbox", { name: "Head bbbbbbbb" });
  await diff.scrollIntoViewIfNeeded();
  await expect(diff).toBeInViewport();
  await page.screenshot({ path: test.info().outputPath("mobile-diff.png") });
  const geometry = await diff.evaluate((element) => {
    const r = element.getBoundingClientRect();
    return {
      x: r.x,
      y: r.y,
      width: r.width,
      height: r.height,
      viewportHeight: innerHeight,
    };
  });
  console.log("mobile diff geometry", JSON.stringify(geometry));
  expect(
    Math.min(geometry.y + geometry.height, geometry.viewportHeight) -
      Math.max(geometry.y, 0),
  ).toBeGreaterThan(180);
});

test("failed refresh preserves read-only review and recovers", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=rich");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.evaluate(() => window.__reviewFixture?.fail());
  await expect(
    page
      .getByRole("alert")
      .filter({ hasText: "Showing the last loaded state read-only" }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Review canvas" }),
  ).toHaveAttribute("data-read-only", "true");
  await page.evaluate(() => window.__reviewFixture?.recover());
  await expect(
    page.getByRole("region", { name: "Review canvas" }),
  ).toHaveAttribute("data-read-only", "false");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
});

test("line comment draft survives candidate change and read-only recovery", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=rich");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.getByText("Review", { exact: true }).first().click();
  const head = page.getByRole("textbox", { name: "Head bbbbbbbb" });
  await expect(head).toBeVisible();
  await head.focus();
  await page.keyboard.press("Control+Alt+m");
  const draft = page.getByRole("textbox", {
    name: "New comment on src/parse.ts",
  });
  await draft.fill("Unsent line draft");
  await page.getByRole("combobox", { name: "Candidate revision" }).click();
  await page.getByRole("option", { name: /^#1 / }).click();
  await page.getByRole("button", { name: "Back to current" }).click();
  await expect(draft).toHaveValue("Unsent line draft");
  await page.evaluate(() => window.__reviewFixture?.fail());
  await expect(
    page.getByRole("region", { name: "Review canvas" }),
  ).toHaveAttribute("data-read-only", "true");
  await expect(draft).toHaveValue("Unsent line draft");
  await page.evaluate(() => window.__reviewFixture?.recover());
  await expect(
    page.getByRole("region", { name: "Review canvas" }),
  ).toHaveAttribute("data-read-only", "false");
  await expect(draft).toHaveValue("Unsent line draft");
});

test("range comment retries the identical unknown-outcome request", async ({
  page,
}) => {
  await page.goto("/tests/e2e/review-canvas.html?tour=rich");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.getByText("Review", { exact: true }).first().click();
  const requests: Record<string, unknown>[] = [];
  await page.exposeFunction(
    "captureSmokeComment",
    (request: Record<string, unknown>) => requests.push(request),
  );
  await page.evaluate(() => {
    const fetch = window.fetch.bind(window);
    let failed = false;
    window.fetch = async (input, init) => {
      const request =
        input instanceof Request ? input : new Request(input, init);
      if (new URL(request.url).pathname.endsWith("/comments")) {
        const body: unknown = await request.clone().json();
        const capture = (
          window as unknown as {
            captureSmokeComment: (value: unknown) => Promise<void>;
          }
        ).captureSmokeComment;
        await capture(body);
        if (!failed) {
          failed = true;
          return new Response(
            JSON.stringify({ detail: "Synthetic transient save failure" }),
            { status: 503, headers: { "Content-Type": "application/json" } },
          );
        }
      }
      return fetch(input, init);
    };
  });
  const head = page.getByRole("textbox", { name: "Head bbbbbbbb" });
  await head.focus();
  await page.keyboard.press("Control+Home");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowRight");
  await page
    .getByRole("button", { name: "Comment on Head L1–3", exact: true })
    .click();
  const draft = page.getByRole("textbox", {
    name: "New comment on src/parse.ts",
  });
  await draft.fill("Retry this exact selected range");
  await page.getByRole("button", { name: "Comment", exact: true }).click();
  await expect.poll(() => requests.length).toBe(1);
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toBeEnabled();
  await expect(draft).toHaveValue("Retry this exact selected range");
  expect(requests).toHaveLength(1);
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect.poll(() => requests.length).toBe(2);
  expect(requests[1]).toEqual(requests[0]);
  expect(requests[0]).toMatchObject({
    anchor: {
      position: {
        candidate_id: "c2",
        path: "src/parse.ts",
        side: "head",
        start_line: 1,
        end_line: 3,
      },
    },
  });
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__reviewFixture?.comments()))?.length,
    )
    .toBe(1);
  await expect(
    page
      .locator("[data-review-thread-id]:visible")
      .getByText("Retry this exact selected range", { exact: true })
      .first(),
  ).toBeVisible();
});
