import { expect, test } from "@playwright/test";
import { openReviewFixture } from "./reviewFixtureReady";

test.beforeEach(async ({ page }) => {
  await openReviewFixture(page, "/tests/e2e/review-canvas.html?tour=rich");
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible({ timeout: 15000 });
});

test("negative HTML anchor index is inert while its valid sibling reveals code", async ({
  page,
}) => {
  await page.locator('[data-block-id="visual"]').scrollIntoViewIfNeeded();
  const frame = page.frameLocator('iframe[title="Tour visual"]');
  await frame.locator("#invalid-anchor").click();
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeAttached();
  await expect(
    page.getByRole("radio", { name: "Tour", exact: true }),
  ).toBeChecked();
  await frame.locator("#valid-anchor").click();
  await expect(page.getByRole("radio", { name: /^Changes/ })).toBeChecked();
  await expect(
    page.getByRole("textbox", { name: "Head bbbbbbbb" }),
  ).toBeVisible();
});

for (const version of [1, 2]) {
  test(`chat reveal selects exact cross-topic version ${version} and second discussion`, async ({
    page,
  }) => {
    await openReviewFixture(
      page,
      "/tests/e2e/review-canvas.html?tour=rich&navigation=exact",
    );
    await expect(
      page.getByRole("heading", { name: "Input trimming" }),
    ).toBeVisible();
    await page.evaluate(
      (selected) =>
        window.dispatchEvent(
          new CustomEvent("refact:review-reveal", {
            detail: {
              reviewId: "fixture-review",
              threadId: `navigation-${selected}-exact`,
            },
          }),
        ),
      version,
    );
    const topic = page.locator('[data-step-id="step-2"]');
    await expect(topic.getByRole("combobox")).toContainText(
      `v${version} · ${version === 2 ? "latest" : "historical"}`,
    );
    const discussion = topic.locator(
      `[data-review-thread-id="navigation-${version}-exact"]`,
    );
    await expect(discussion).toBeVisible();
    await expect
      .poll(async () =>
        discussion.evaluate((element) => {
          const reader = element.closest("[data-review-document]");
          if (!reader) return Number.POSITIVE_INFINITY;
          const target = element.getBoundingClientRect();
          const viewport = reader.getBoundingClientRect();
          // Near the document end scrollTop is clamped. Require the exact
          // second discussion inside the reader viewport, not exact top=0.
          return target.top >= viewport.top - 2 &&
            target.bottom <= viewport.bottom + 2
            ? 0
            : 100;
        }),
      )
      .toBeLessThan(5);
    if (version === 1) {
      await topic.getByRole("button", { name: /Apply updated topic/ }).click();
      await expect(topic.getByRole("combobox")).toContainText("v2 · latest");
      await page.evaluate(() => window.__reviewFixture?.publishTopic());
      await expect(
        page
          .getByRole("list", { name: "Tour steps" })
          .getByRole("button", { name: "Later investigation" }),
      ).toBeAttached();
      await expect(topic.getByRole("combobox")).toContainText("v2 · latest");
    }
  });
}

test("native lazy mount and topic refresh preserve the document reader position", async ({
  page,
}) => {
  await openReviewFixture(
    page,
    "/tests/e2e/review-canvas.html?tour=rich&reader=delayed",
  );
  const reader = page.locator("[data-review-document]");
  const code = page.locator('[data-block-id="code"]');
  await code.scrollIntoViewIfNeeded();
  await expect(code.locator(".cm-editor")).toHaveCount(0);
  // Record every frame, not just the eventual position: a jump followed by
  // restoration is still disruptive to someone reading the document.
  const before = await reader.evaluate((element) => {
    const node = element as HTMLElement & {
      positions?: number[];
      sampling?: boolean;
    };
    node.positions = [node.scrollTop];
    node.sampling = true;
    const sample = () => {
      node.positions?.push(node.scrollTop);
      if (node.sampling) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
    return node.scrollTop;
  });
  expect(before).toBeGreaterThan(0);
  await page.evaluate(() => window.__reviewFixture?.releaseFiles());
  await expect(code.locator(".cm-editor")).toHaveCount(1);
  await page.evaluate(() => window.__reviewFixture?.updateTopic());
  await expect(
    page.getByRole("button", { name: /Apply updated topic/ }),
  ).toBeAttached();
  // Give CodeMirror measurement writes two animation frames to settle.
  await reader.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  const positions = await reader.evaluate((element) => {
    const node = element as HTMLElement & {
      positions?: number[];
      sampling?: boolean;
    };
    node.sampling = false;
    return node.positions ?? [];
  });
  expect(
    Math.max(...positions.map((position) => Math.abs(position - before))),
  ).toBeLessThanOrEqual(2);
});

test("living topics remain visible and updates are offered explicitly", async ({
  page,
}) => {
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  const pages = page.getByRole("list", { name: "Tour steps" });
  await expect(
    pages.getByRole("button", { name: "Loop bound fix" }),
  ).toBeAttached();
  await page.evaluate(() => window.__reviewFixture?.publishTopic());
  await expect(
    pages.getByRole("button", { name: "Later investigation" }),
  ).toBeAttached();
  await page.evaluate(() => window.__reviewFixture?.updateTopic());
  await expect(
    page.getByRole("button", { name: /Apply updated topic/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Apply updated topic/ }).click();
  await expect(
    page.getByRole("heading", { name: "Updated input trimming" }),
  ).toBeVisible();
});

test("interactive HTML survives a same-content topic update", async ({
  page,
}) => {
  await page.locator('[data-block-id="visual"]').scrollIntoViewIfNeeded();
  const frame = page.frameLocator('iframe[title="Tour visual"]');
  const counter = frame.locator("#rich-counter");
  await expect(counter).toHaveText("0");
  await counter.click();
  await expect(counter).toHaveText("1");
  const iframe = page.locator('iframe[title="Tour visual"]');
  const original = await iframe.elementHandle();
  await page
    .getByRole("radio", { name: /^Changes/ })
    .locator("..")
    .click();
  await page.getByText("Tour", { exact: true }).first().click();
  expect(
    await original?.evaluate(
      (element) =>
        element === document.querySelector('iframe[title="Tour visual"]'),
    ),
  ).toBe(true);
  await expect(counter).toHaveText("1");
  await page.evaluate(() => window.__reviewFixture?.updateTopic());
  await page.getByRole("button", { name: /Apply updated topic/ }).click();
  await expect(counter).toHaveText("1");
  expect(
    await counter.evaluate(
      (element) =>
        (
          element.ownerDocument.defaultView as Window & {
            visualInitializations?: number;
          }
        ).visualInitializations,
    ),
  ).toBe(1);
});

test("source inventory distinguishes consulted files from fixture execution", async ({
  page,
}) => {
  await expect(
    page.getByText("Fixture record only; no process was executed").first(),
  ).toBeAttached();
  await expect(
    page.getByRole("link", { name: "https://example.com/reference" }).first(),
  ).toBeAttached();
  await expect(
    page.getByRole("button", { name: "README.md" }).first(),
  ).toBeAttached();
});

test("one header and a keyboard and pointer adjustable split preserve the chat draft", async ({
  page,
}) => {
  const canvas = page.getByRole("region", { name: "Review canvas" });
  await expect(canvas.locator(":scope > header")).toHaveCount(1);
  const chat = page.getByRole("textbox", { name: "Chat composer" });
  await chat.fill("Unsent chat draft");
  const handle = page.getByRole("separator", { name: "Resize chat" });
  await expect(handle).toBeVisible();
  const before = Number(await handle.getAttribute("aria-valuenow"));
  await handle.focus();
  await page.keyboard.press("ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", String(before + 10));
  const box = await handle.boundingBox();
  if (!box) throw new Error("Split handle has no layout box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 35, box.y + box.height / 2);
  await page.mouse.up();
  await expect
    .poll(async () => Number(await handle.getAttribute("aria-valuenow")))
    .toBeGreaterThan(before + 10);
  await handle.focus();
  await page.keyboard.press("Home");
  await expect(handle).toHaveAttribute(
    "aria-valuenow",
    (await handle.getAttribute("aria-valuemin")) ?? "",
  );
  await page.keyboard.press("End");
  await expect(handle).toHaveAttribute(
    "aria-valuenow",
    (await handle.getAttribute("aria-valuemax")) ?? "",
  );
  await expect(chat).toHaveValue("Unsent chat draft");
});

test("source chip opens its exact unchanged file on the current candidate", async ({
  page,
}) => {
  const source = page.locator('[data-block-id="sources"]');
  await source.scrollIntoViewIfNeeded();
  await source.getByRole("button", { name: "README.md" }).click();
  await expect(page.getByTitle("README.md").last()).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Head bbbbbbbb" }),
  ).toContainText("Parser now trims whitespace.");
});

test("block question sends versioned target and replies to the same thread", async ({
  page,
}) => {
  const block = page.locator('[data-block-id="intro"]');
  await block
    .getByRole("button", { name: "Ask about this block", exact: true })
    .click();
  await block
    .getByRole("textbox", { name: "Question about this block" })
    .fill("Why trim before conversion?");
  await block.getByRole("button", { name: "Ask", exact: true }).click();
  await expect(block.getByText("Why trim before conversion?")).toBeVisible();
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__reviewFixture?.comments()))?.length,
    )
    .toBe(1);
  expect(
    (await page.evaluate(() => window.__reviewFixture?.comments()))?.[0],
  ).toMatchObject({
    body: "Why trim before conversion?",
    target: {
      candidate_id: "c2",
      step_id: "step-1",
      version: 2,
      block_id: "intro",
      block_revision: 1,
    },
  });
  await block.getByRole("button", { name: "Reply" }).click();
  await block
    .getByRole("textbox", { name: "Reply" })
    .fill("Please include the edge case.");
  await block.getByRole("button", { name: "Comment", exact: true }).click();
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__reviewFixture?.comments()))?.length,
    )
    .toBe(2);
  await expect(block.getByText("Please include the edge case.")).toBeVisible();
  expect(
    (await page.evaluate(() => window.__reviewFixture?.comments()))?.[1],
  ).toMatchObject({
    body: "Please include the edge case.",
    thread_id: "thread-13",
  });
});
