import { expect, test, type Locator, type Page } from "@playwright/test";

// All transport data and agent replies in this suite are synthetic fixture data.
// These screenshots demonstrate production UI behaviour, not actual backend proof.
async function open(page: Page, width = 1600, theme = "light") {
  await page.setViewportSize({ width, height: 900 });
  await page.emulateMedia({
    colorScheme: theme as "light" | "dark",
    reducedMotion: "reduce",
  });
  await page.goto(
    `/tests/e2e/review-canvas.html?tour=rich&polish=1&theme=${theme}`,
  );
  if (width <= 680) {
    await page.getByRole("radio", { name: "Canvas", exact: true }).focus();
    await page.keyboard.press("Space");
  }
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
}
async function shot(page: Page, name: string) {
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: test.info().outputPath(`${name}.png`) });
}
async function keyboardToggle(page: Page, control: Locator, expanded: boolean) {
  await control.scrollIntoViewIfNeeded();
  await control.focus();
  await expect(control).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(control).toHaveAttribute("aria-expanded", String(expanded));
  const id = await control.getAttribute("aria-controls");
  expect(id).toBeTruthy();
  await expect(page.locator(`[id="${id}"]`)).toBeAttached();
}
const block = (page: Page, id: string) =>
  page.locator(`[data-step-id="step-1"] [data-block-id="${id}"]`).first();

async function showLiveVisual(page: Page, visual: Locator) {
  await visual.scrollIntoViewIfNeeded();
  // Legacy HTML has no authored fallback: reduced motion exposes inert source
  // and must not start its scripts until this visual is explicitly opted in.
  await expect(visual.locator("iframe")).toHaveCount(0);
  const explanation = visual.getByRole("region", {
    name: "Static explanation",
    exact: true,
  });
  await expect(explanation).toBeVisible();
  await expect(explanation).toContainText("live scripts have not started");
  await expect(explanation.locator("pre")).toContainText("rich-counter");
  await expect(explanation.locator("script, iframe, button")).toHaveCount(0);
  const live = visual.getByRole("button", {
    name: "Show live explanation",
    exact: true,
  });
  await live.focus();
  await expect(live).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(visual.locator('iframe[title="Tour visual"]')).toBeVisible();
  await expect(explanation).toHaveCount(0);
}

for (const width of [1600, 768, 390]) {
  for (const theme of ["light", "dark"]) {
    test(`synthetic diagram, dynamic state and draft ${width} ${theme}`, async ({
      page,
    }) => {
      await open(page, width, theme);
      const intro = block(page, "intro");
      await expect(intro.locator("svg").first()).toBeVisible({
        timeout: 15000,
      });
      await shot(page, "01-diagram-before");
      const visual = block(page, "visual");
      await showLiveVisual(page, visual);
      const frame = page.frameLocator('iframe[title="Tour visual"]');
      await frame.locator("#rich-counter").click();
      await expect(frame.locator("#rich-counter")).toHaveText("1");
      await frame.locator("#dynamic-toggle").focus();
      await page.keyboard.press("Enter");
      await expect(frame.locator("#dynamic-toggle")).toHaveAttribute(
        "aria-expanded",
        "true",
      );
      await expect(frame.locator("#dynamic-panel")).toBeVisible();
      await shot(page, "02-dynamic-after");
      const question = visual.getByRole("button", {
        name: "Ask about this block",
        exact: true,
      });
      await keyboardToggle(page, question, true);
      const draft = visual.getByRole("textbox", {
        name: "Question about this block",
      });
      await draft.fill("Synthetic unsent question survives disclosure");
      const original = await draft.elementHandle();
      await keyboardToggle(page, question, false);
      await keyboardToggle(page, question, true);
      await expect(draft).toHaveValue(
        "Synthetic unsent question survives disclosure",
      );
      expect(await original?.evaluate((node) => node.isConnected)).toBe(true);
      await shot(page, "03-question-draft-after");
      if (width <= 768) {
        const browse = page.getByRole("button", { name: "Browse topics" });
        await browse.click();
        await page
          .getByRole("list", { name: "Tour steps" })
          .getByRole("button")
          .first()
          .click();
        await expect(browse).toBeFocused();
      }
    });
  }
}

test("synthetic canonical human comment, agent rich reply and reply composer", async ({
  page,
}) => {
  await open(page);
  const thread = page
    .locator('[data-review-thread-id="synthetic-rich-thread"]')
    .first();
  await thread.scrollIntoViewIfNeeded();
  await expect(thread.locator('[data-actor="user"]')).toContainText(
    "Synthetic human",
  );
  await expect(thread.locator('[data-actor="agent"]')).toContainText(
    "Synthetic agent explanation",
  );
  await shot(page, "canonical-before");
  const fullReply = thread.getByRole("button", {
    name: /Show full reply|Show less/,
  });
  await keyboardToggle(page, fullReply, true);
  await keyboardToggle(page, fullReply, false);
  await thread.getByRole("button", { name: "Reply", exact: true }).click();
  await expect(
    thread.getByRole("button", { name: "Comment", exact: true }),
  ).toBeDisabled();
  await thread
    .getByRole("textbox", { name: "Reply", exact: true })
    .fill("Synthetic human follow-up");
  await thread.getByRole("button", { name: "Comment", exact: true }).click();
  await expect(
    thread.getByText("Synthetic human follow-up", { exact: true }),
  ).toBeVisible();
  expect(
    (await page.evaluate(() => window.__reviewFixture?.comments()))?.[0],
  ).toMatchObject({
    thread_id: "synthetic-rich-thread",
    body: "Synthetic human follow-up",
  });
  await page.evaluate(() => window.__reviewFixture?.syntheticReply());
  await expect(thread.getByText(/Synthetic live agent reply:/)).toBeVisible();
  await keyboardToggle(
    page,
    thread.getByRole("button", { name: "Topic history", exact: true }),
    true,
  );
  await expect(page.getByText("No earlier versions.")).toBeVisible();
  await shot(page, "canonical-after");
  const intro = block(page, "intro");
  await keyboardToggle(
    page,
    intro.getByRole("button", { name: "Ask about this block", exact: true }),
    true,
  );
  await expect(
    intro.getByRole("button", { name: "Ask", exact: true }),
  ).toBeDisabled();
  await intro
    .getByRole("textbox", { name: "Question about this block" })
    .fill("Synthetic posted block question");
  await intro.getByRole("button", { name: "Ask", exact: true }).click();
  await expect
    .poll(
      async () =>
        (await page.evaluate(() => window.__reviewFixture?.comments()))?.length,
    )
    .toBe(2);
  await expect(
    intro.getByText("Synthetic posted block question", { exact: true }),
  ).toBeVisible();
  expect(
    (await page.evaluate(() => window.__reviewFixture?.comments()))?.[1],
  ).toMatchObject({
    body: "Synthetic posted block question",
    target: {
      candidate_id: "c2",
      step_id: "step-1",
      version: 2,
      block_id: "intro",
      block_revision: 1,
    },
  });
});

test("native code, diff, metadata, source and evidence controls", async ({
  page,
}) => {
  await open(page);
  const code = block(page, "code");
  await code.scrollIntoViewIfNeeded();
  await expect(code.locator(".cm-editor")).toHaveCount(1);
  await shot(page, "native-before");
  await keyboardToggle(
    page,
    code.getByRole("button", { name: "Reference metadata", exact: true }),
    true,
  );
  await expect(code.getByText("digest-c2", { exact: true })).toBeVisible();
  const diff = block(page, "diff");
  await diff.scrollIntoViewIfNeeded();
  await diff.getByRole("button", { name: "Unified diff", exact: true }).click();
  await expect(
    diff.getByRole("button", { name: "Split diff", exact: true }),
  ).toBeVisible();
  await diff.getByRole("button", { name: "Split diff", exact: true }).click();
  const sources = block(page, "sources");
  await sources.scrollIntoViewIfNeeded();
  await expect(
    sources.getByRole("link", { name: "https://example.com/reference" }),
  ).toHaveAttribute("rel", /noreferrer/);
  const evidence = block(page, "evidence");
  await evidence.scrollIntoViewIfNeeded();
  await expect(evidence).toContainText(
    "Fixture record only; no process was executed",
  );
  await shot(page, "evidence-after");
  await sources.getByRole("button", { name: "README.md", exact: true }).click();
  await expect(
    page.getByRole("radio", { name: "Review", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("textbox", { name: "Head bbbbbbbb" }),
  ).toContainText("Parser now trims whitespace.");
});

test("worker, blob and history disclosures expose keyboard state and cancel disables", async ({
  page,
}) => {
  await open(page);
  const workers = page.getByRole("button", { name: /Workers \(/ });
  await keyboardToggle(page, workers, true);
  await shot(page, "workers-before");
  const cancel = page.getByRole("button", {
    name: "Cancel Probe empty-input behaviour",
  });
  await cancel.focus();
  await page.keyboard.press("Enter");
  await expect(cancel).toBeDisabled();
  await expect(page.getByText("Stopping…")).toBeVisible();
  await keyboardToggle(page, workers, false);
  const sources = page.getByRole("button", {
    name: "Sources & Evidence",
    exact: true,
  });
  await keyboardToggle(page, sources, true);
  await keyboardToggle(
    page,
    page.getByRole("button", { name: /^(Show|Hide) stdout/ }).first(),
    true,
  );
  await expect(page.getByText(/PASS parse.test.ts/).first()).toBeVisible();
  await keyboardToggle(
    page,
    page.getByRole("button", { name: /^(Show|Hide) stderr/ }).first(),
    true,
  );
  await keyboardToggle(
    page,
    page.getByRole("button", { name: /Show details|Show less/ }).first(),
    true,
  );
  await keyboardToggle(
    page,
    page.getByRole("button", { name: /^(Show|Hide) artifact 1/ }).first(),
    true,
  );
  await shot(page, "sources-blobs-after");
  await expect(page.getByRole("heading", { name: "Execution" })).toBeVisible();
  await expect(
    page.getByRole("article", { name: "Evidence: External reference" }),
  ).toBeVisible();
  await expect(
    page.getByRole("article", { name: /Evidence: Synthetic source context/ }),
  ).toBeVisible();
  const material = page.getByRole("button", {
    name: "Test run workspace",
    exact: true,
  });
  await keyboardToggle(page, material, false);
  await keyboardToggle(page, material, true);
  await keyboardToggle(page, sources, false);
  await keyboardToggle(page, workers, true);
  const worker = page.getByRole("article", {
    name: "Worker: Run parser tests",
  });
  await expect(worker).toContainText("Completed");
  await expect(worker).toContainText("agent-verify");
  await expect(worker.getByRole("button", { name: /Cancel/ })).toHaveCount(0);
});

test("pending update, explicit apply and historical version use shared select", async ({
  page,
}) => {
  await open(page);
  await shot(page, "update-before");
  await page.evaluate(() => window.__reviewFixture?.syntheticNewVersion());
  const apply = page.getByRole("button", { name: /Apply updated topic/ });
  await expect(apply).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await shot(page, "update-pending");
  await apply.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("heading", { name: "Updated input trimming" }),
  ).toBeVisible();
  const version = page.getByRole("combobox", {
    name: "Version of Updated input trimming",
  });
  await version.click();
  await page.getByRole("option", { name: /v2/ }).click();
  await expect(
    page.getByRole("heading", { name: "Input trimming" }),
  ).toBeVisible();
  await shot(page, "update-historical");
  await page.getByRole("button", { name: /Apply updated topic/ }).click();
  await page.evaluate(() => window.__reviewFixture?.publishTopic());
  await expect(
    page.getByRole("heading", { name: "Later investigation" }),
  ).toBeAttached();
});

test("resize, disclosure and update preserve iframe, editor, draft and reader identity", async ({
  page,
}) => {
  await open(page);
  const chat = page.getByRole("textbox", { name: "Chat composer" });
  await chat.fill("Synthetic persistent chat draft");
  const code = block(page, "code");
  await code.scrollIntoViewIfNeeded();
  await expect(code.locator(".cm-editor")).toHaveCount(1);
  const visual = block(page, "visual");
  await showLiveVisual(page, visual);
  const counter = page
    .frameLocator('iframe[title="Tour visual"]')
    .locator("#rich-counter");
  await counter.click();
  const reader = page.locator("[data-review-document]");
  const identities = await Promise.all([
    page.locator('iframe[title="Tour visual"]').elementHandle(),
    code.locator(".cm-editor").elementHandle(),
    chat.elementHandle(),
    reader.elementHandle(),
  ]);
  const scroll = await reader.evaluate((node) => node.scrollTop);
  await page.evaluate(() => window.__reviewFixture?.syntheticNewVersion());
  await expect(
    page.getByRole("button", { name: /Apply updated topic/ }),
  ).toBeAttached();
  expect(
    Math.abs((await reader.evaluate((node) => node.scrollTop)) - scroll),
  ).toBeLessThanOrEqual(2);
  await page.setViewportSize({ width: 390, height: 900 });
  await page.getByRole("radio", { name: "Canvas", exact: true }).focus();
  await page.keyboard.press("Space");
  await page.setViewportSize({ width: 1600, height: 900 });
  await expect(counter).toHaveText("1");
  await expect(chat).toHaveValue("Synthetic persistent chat draft");
  for (const identity of identities)
    expect(await identity?.evaluate((node) => node.isConnected)).toBe(true);
  await page.getByRole("button", { name: /Apply updated topic/ }).click();
  await expect(counter).toHaveText("1");
  for (const identity of identities)
    expect(await identity?.evaluate((node) => node.isConnected)).toBe(true);
  await shot(page, "identities-after");
});

test("read-only error recovery and native anchor handoff remain operable", async ({
  page,
}) => {
  await open(page, 390, "dark");
  const intro = block(page, "intro");
  await keyboardToggle(
    page,
    intro.getByRole("button", { name: "Ask about this block", exact: true }),
    true,
  );
  const draft = intro.getByRole("textbox", {
    name: "Question about this block",
  });
  await draft.fill("Synthetic recovery draft");
  await page.evaluate(() => window.__reviewFixture?.fail());
  const canvas = page.getByRole("region", { name: "Review canvas" });
  await expect(canvas).toHaveAttribute("data-read-only", "true");
  await expect(draft).toBeDisabled();
  await shot(page, "error-before-recovery");
  await page.evaluate(() => window.__reviewFixture?.recover());
  await expect(canvas).toHaveAttribute("data-read-only", "false");
  await expect(draft).toHaveValue("Synthetic recovery draft");
  await showLiveVisual(page, block(page, "visual"));
  const frame = page.frameLocator('iframe[title="Tour visual"]');
  await frame.locator("#invalid-anchor").click();
  await expect(
    page.getByRole("radio", { name: "Tour", exact: true }),
  ).toBeChecked();
  await frame.locator("#valid-anchor").click();
  await expect(
    page.getByRole("radio", { name: "Review", exact: true }),
  ).toBeChecked();
  await expect(
    page.getByRole("textbox", { name: "Head bbbbbbbb" }),
  ).toBeVisible();
  await shot(page, "recovered-native-handoff");
});
