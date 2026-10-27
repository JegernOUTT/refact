import { expect, test, type Locator, type Page } from "@playwright/test";
const route = "/tests/e2e/route-showcase.html?route=queue";
async function chooseTiming(page: Page, item: Locator, label: string) {
  await item.getByRole("combobox").click();
  await page.getByRole("option", { name: new RegExp(`^${label}`) }).click();
}
/** Rows wrap the preview under the title below 360px, so the 4-row cap is taller there. */
function fourRowCap(width: number): [number, number] {
  return width <= 360 ? [180, 200] : [140, 150];
}
async function expectBounded(page: Page, width: number) {
  const bounds = await page.getByTestId("queued-item").evaluateAll((elements) =>
    elements.map((element) => ({
      left: element.getBoundingClientRect().left,
      right: element.getBoundingClientRect().right,
      overflow: element.scrollWidth > element.clientWidth,
    })),
  );
  for (const bound of bounds) {
    expect(bound.left).toBeGreaterThanOrEqual(0);
    expect(bound.right).toBeLessThanOrEqual(width);
    expect(bound.overflow).toBe(false);
  }
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
}
for (const width of [330, 1280]) {
  test(`queue controls and mock boundaries at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(route);
    await expect(page.getByTestId("queued-item")).toHaveCount(5);
    expect(
      await page
        .getByTestId("queued-item")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-delivery-id")),
        ),
    ).toEqual([
      "queued-user-1",
      "delivery-agent-1",
      "delivery-process-1",
      "delivery-cron-1",
      "delivery-unlabeled-1",
    ]);
    await expectBounded(page, width);
    const agent = page.locator('[data-delivery-id="delivery-agent-1"]');
    const process = page.locator('[data-delivery-id="delivery-process-1"]');
    await expect(process).toHaveAttribute("data-push", "preempt");
    await expect(process.getByRole("combobox")).toHaveAccessibleName(
      /Interrupt now/,
    );
    await chooseTiming(page, agent, "When idle");
    await expect(agent).toHaveAttribute("data-push", "when_idle");
    await expect(agent.getByRole("combobox")).toHaveAccessibleName(/When idle/);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window as unknown as { __routeShowcaseCommands: unknown[] })
              .__routeShowcaseCommands,
        ),
      )
      .toContainEqual({
        type: "update_pending_delivery",
        delivery_id: "delivery-agent-1",
        push: "when_idle",
        client_request_id: expect.any(String),
      });
    await chooseTiming(page, agent, "After current step");
    await page
      .getByRole("button", { name: "Mock: assistant finished" })
      .click();
    await expect(agent).toHaveCount(1);
    await page.getByRole("button", { name: "Mock: tools finished" }).click();
    await expect(agent).toHaveCount(0);
    const cron = page.locator('[data-delivery-id="delivery-cron-1"]');
    await expect(cron).toHaveCount(1);
    await page
      .getByRole("button", { name: "Mock: interrupt and discard" })
      .click();
    await expect(process).toHaveCount(0);
    await expect(cron).toHaveCount(1);
    await page.getByRole("button", { name: "Mock: final idle" }).click();
    await expect(page.getByTestId("queued-item")).toHaveCount(0);
    await expect(
      page.getByTestId("event-row").filter({
        hasText:
          "Active step cancelled; partial assistant and tool output discarded.",
      }),
    ).toBeVisible();
    await expectBounded(page, width);
  });
}
test("cancel delivery and edit a legacy priority message", async ({ page }) => {
  await page.goto(route);
  const agent = page.locator('[data-delivery-id="delivery-agent-1"]');
  await agent.getByRole("button", { name: /Cancel pending delivery/ }).click();
  await expect(agent).toHaveCount(0);
  const user = page.locator('[data-delivery-id="queued-user-1"]');
  await chooseTiming(page, user, "Send next");
  await expect(user).toHaveAttribute("data-push", "preempt");
  await user
    .getByRole("button", { name: "Click to edit queued message" })
    .click();
  await expect(user).toHaveCount(0);
});
for (const theme of ["light", "dark"]) {
  for (const width of [330, 768, 1280]) {
    test(`dense queue and inline events ${theme} ${width}`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: 1000 });
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto(`${route}&queue_count=6&theme=${theme}&waiting=1`);
      await expect(page.getByTestId("queued-item")).toHaveCount(6);
      await expect(page.getByTestId("queue-header")).toContainText(
        "delivering now",
      );
      const list = page.getByTestId("queue-list");
      const geometry = await list.evaluate((element) => ({
        height: element.clientHeight,
        scroll: element.scrollHeight,
        overflow: getComputedStyle(element).overflowY,
      }));
      const [minCap, maxCap] = fourRowCap(width);
      expect(geometry.height).toBeGreaterThanOrEqual(minCap);
      expect(geometry.height).toBeLessThanOrEqual(maxCap);
      expect(geometry.scroll).toBeGreaterThan(geometry.height);
      expect(geometry.overflow).toBe("auto");
      await expect(list).toHaveAttribute("data-overflow", "true");
      await expectBounded(page, width);
      // Only the tail is stable in the virtualized transcript.
      await expect(
        page
          .locator('[data-testid="event-row"][data-subkind="system_notice"]')
          .last(),
      ).toBeVisible();
      const agent = page.locator('[data-delivery-id="delivery-agent-1"]');
      const select = agent.getByRole("combobox");
      await select.focus();
      await page.keyboard.press("Space");
      await page.getByRole("option", { name: /^When idle/ }).click();
      await expect(select).toBeFocused();
      await expect(agent).toHaveAttribute("data-push", "when_idle");
      await agent.getByRole("button", { name: /^\d+ Agent completed/ }).click();
      await expect(
        agent.getByRole("radio", { name: "When idle" }),
      ).toBeChecked();
      await expect(list).toHaveAttribute("data-expanded-row", "true");
      const rowBounds = await agent.boundingBox();
      const listBounds = await list.boundingBox();
      expect(rowBounds!.y).toBeGreaterThanOrEqual(listBounds!.y);
      expect(rowBounds!.y + rowBounds!.height).toBeLessThanOrEqual(
        listBounds!.y + listBounds!.height + 1,
      );
      await expectBounded(page, width);
      await page.screenshot({
        path: testInfo.outputPath(`expanded-${theme}-${width}.png`),
        fullPage: true,
      });
      await page
        .getByRole("button", { name: "Collapse the delivery queue" })
        .click();
      await expect(list).toHaveCount(0);
      await page
        .getByRole("button", { name: /Expand the delivery queue/ })
        .click();
      await list.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
      await expect(list).toHaveAttribute("data-overflow", "false");
    });
  }
}
for (const width of [330, 1280]) {
  test(`100 long deliveries remain bounded ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${route}&queue_count=100`);
    await expect(page.getByTestId("queued-item")).toHaveCount(100);
    await expectBounded(page, width);
    expect(
      (await page.getByTestId("queue-list").boundingBox())!.height,
    ).toBeLessThanOrEqual(fourRowCap(width)[1]);
    const last = page.locator('[data-delivery-id="delivery-stress-100"]');
    await last.scrollIntoViewIfNeeded();
    await expect(last).toBeInViewport();
    expect(
      await last.evaluate((element) => element.parentElement!.scrollTop),
    ).toBeGreaterThan(0);
    await chooseTiming(page, last, "When idle");
    await expect(last).toHaveAttribute("data-push", "when_idle");
    await last.getByRole("button", { name: /Cancel pending delivery/ }).click();
    await expect(last).toHaveCount(0);
    await expect(page.getByTestId("queued-item")).toHaveCount(99);
  });
  test(`HTTP errors retain timing and keyboard access ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${route}&queue_error=1`);
    const item = page.locator('[data-delivery-id="delivery-agent-1"]');
    await chooseTiming(page, item, "When idle");
    await expect(item.getByRole("alert")).toHaveText(
      "Could not change delivery timing. Try choosing it again.",
    );
    await expect(item).toHaveAttribute("data-push", "append");
    await expect(item.getByRole("combobox")).toHaveAccessibleName(
      /After current step/,
    );
    const cancel = item.getByRole("button", {
      name: /Cancel pending delivery/,
    });
    await cancel.focus();
    await page.keyboard.press("Enter");
    await expect(item.getByRole("alert")).toHaveText(
      "Could not update this queued item. Try the control again.",
    );
    await expect(cancel).toBeEnabled();
  });
  test(`pending timing disables duplicate actions ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${route}&queue_pending=1`);
    const item = page.locator('[data-delivery-id="delivery-agent-1"]');
    await chooseTiming(page, item, "When idle");
    await expect(item).toHaveAttribute("aria-busy", "true");
    await expect(item.getByRole("combobox")).toBeDisabled();
    await expect(
      item.getByRole("button", { name: /Cancel pending delivery/ }),
    ).toBeDisabled();
    expect(
      await page.evaluate(
        () =>
          (window as unknown as { __routeShowcaseCommands: unknown[] })
            .__routeShowcaseCommands.length,
      ),
    ).toBe(1);
    await page.evaluate(() =>
      (
        window as unknown as { __releaseQueueRequest: () => void }
      ).__releaseQueueRequest(),
    );
    await expect(item).toHaveAttribute("aria-busy", "false");
    await expect(item).toHaveAttribute("data-push", "when_idle");
  });
}
test("row entry motion honors reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto(route);
  const item = page.getByTestId("queued-item").first();
  await expect(item).toBeVisible();
  expect(
    await item.evaluate((element) =>
      parseFloat(getComputedStyle(element).animationDuration),
    ),
  ).toBeGreaterThan(0);
  await page.emulateMedia({ reducedMotion: "reduce" });
  expect(
    await item.evaluate((element) =>
      parseFloat(getComputedStyle(element).animationDuration),
    ),
  ).toBe(0);
});
type QueueRequest = { path: string; method: string; body: unknown };
async function queueRequests(page: Page) {
  return page.evaluate(() =>
    (
      window as unknown as { __routeShowcaseRequests: QueueRequest[] }
    ).__routeShowcaseRequests.filter(
      (request) =>
        request.path.includes("/queue/") || request.path.endsWith("/commands"),
    ),
  );
}
async function routeRejections(page: Page) {
  return page.evaluate(
    () =>
      (window as unknown as { __routeShowcaseQueueRouteRejections: string[] })
        .__routeShowcaseQueueRouteRejections,
  );
}
async function expectModeOptions(page: Page, item: Locator, names: string[]) {
  await item.getByRole("combobox").click();
  const options = page.getByRole("option");
  await expect(options).toHaveCount(names.length);
  for (const [index, name] of names.entries()) {
    await expect(options.nth(index)).toContainText(name);
  }
  await page.keyboard.press("Escape");
  await expect(options).toHaveCount(0);
}
const commandRows = [
  ["queued-question-1", /submit question response/i],
  ["queued-regenerate-1", /regenerate/i],
  ["queued-tool-1", /tool decision/i],
] as const;
test("mixed queued commands use priority routes, deliveries keep timing", async ({
  page,
}) => {
  await page.goto(`${route}&queue_commands=1`);
  await expect(page.getByTestId("queued-item")).toHaveCount(8);
  for (const [id, title] of commandRows) {
    const item = page.locator(`[data-delivery-id="${id}"]`);
    await expect(item.getByRole("combobox")).toHaveAccessibleName(
      /^Queue priority for /,
    );
    await expectModeOptions(page, item, ["Send next", "In order"]);
    await expect(
      item.getByRole("button", { name: "Click to edit queued message" }),
    ).toHaveCount(0);
    await item
      .getByRole("button", { name: new RegExp(`^\\d+ ${title.source}`, "i") })
      .click();
    await expect(item.getByRole("radio")).toHaveCount(2);
    await expect(item.getByRole("radio", { name: "When idle" })).toHaveCount(0);
    await expect(
      item.getByRole("button", { name: "Edit queued message text" }),
    ).toHaveCount(0);
    await expect(
      item.getByRole("button", { name: "Remove from queue" }),
    ).toBeVisible();
    await item
      .getByRole("button", { name: new RegExp(`^\\d+ ${title.source}`, "i") })
      .click();
  }
  const delivery = page.locator('[data-delivery-id="delivery-agent-1"]');
  await expectModeOptions(page, delivery, [
    "Interrupt now",
    "After current step",
    "When idle",
  ]);

  const question = page.locator('[data-delivery-id="queued-question-1"]');
  await chooseTiming(page, question, "Send next");
  await expect(question).toHaveAttribute("data-push", "preempt");
  const regenerate = page.locator('[data-delivery-id="queued-regenerate-1"]');
  await expect(regenerate).toHaveAttribute("data-push", "preempt");
  await chooseTiming(page, regenerate, "In order");
  await expect(regenerate).toHaveAttribute("data-push", "append");
  await page
    .locator('[data-delivery-id="queued-tool-1"]')
    .getByRole("button", { name: /^Cancel queued command: /i })
    .click();
  await expect(page.locator('[data-delivery-id="queued-tool-1"]')).toHaveCount(
    0,
  );
  await question
    .getByRole("button", { name: /^Cancel queued command: /i })
    .click();
  await expect(question).toHaveCount(0);
  await chooseTiming(page, delivery, "When idle");
  await expect(delivery).toHaveAttribute("data-push", "when_idle");
  await delivery
    .getByRole("button", { name: /Cancel pending delivery/ })
    .click();
  await expect(delivery).toHaveCount(0);

  expect(await routeRejections(page)).toEqual([]);
  const requests = await queueRequests(page);
  const queuePath = (id: string) => `/v1/chats/showcase-chat/queue/${id}`;
  expect(requests).toEqual([
    {
      path: queuePath("queued-question-1"),
      method: "PATCH",
      body: { priority: true },
    },
    {
      path: queuePath("queued-regenerate-1"),
      method: "PATCH",
      body: { priority: false },
    },
    { path: queuePath("queued-tool-1"), method: "DELETE", body: null },
    { path: queuePath("queued-question-1"), method: "DELETE", body: null },
    {
      path: "/v1/chats/showcase-chat/commands",
      method: "POST",
      body: expect.objectContaining({
        type: "update_pending_delivery",
        delivery_id: "delivery-agent-1",
        push: "when_idle",
      }),
    },
    {
      path: "/v1/chats/showcase-chat/commands",
      method: "POST",
      body: expect.objectContaining({
        type: "update_pending_delivery",
        delivery_id: "delivery-agent-1",
        cancel: true,
      }),
    },
  ]);
  await expect(page.getByTestId("queued-item")).toHaveCount(5);
});
test("mixed queue mock rejects crossed delivery and command routes", async ({
  page,
}) => {
  await page.goto(`${route}&queue_commands=1`);
  await expect(page.getByTestId("queued-item")).toHaveCount(8);
  const statuses = await page.evaluate(async () => {
    const base = "/v1/chats/showcase-chat";
    const post = (body: unknown) =>
      fetch(`${base}/commands`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
    return Promise.all([
      fetch(`${base}/queue/delivery-agent-1`, { method: "DELETE" }),
      fetch(`${base}/queue/delivery-agent-1`, {
        method: "PATCH",
        body: JSON.stringify({ priority: true }),
      }),
      post({
        type: "update_pending_delivery",
        delivery_id: "queued-question-1",
        cancel: true,
      }),
      post({
        type: "update_pending_delivery",
        delivery_id: "queued-regenerate-1",
        push: "when_idle",
      }),
      fetch(`${base}/queue/queued-tool-1`, {
        method: "PATCH",
        body: JSON.stringify({ push: "when_idle" }),
      }),
    ]).then((responses) => responses.map((response) => response.status));
  });
  expect(statuses).toEqual([409, 409, 409, 409, 409]);
  expect(await routeRejections(page)).toHaveLength(5);
  await expect(page.getByTestId("queued-item")).toHaveCount(8);
  await expect(
    page.locator('[data-delivery-id="queued-regenerate-1"]'),
  ).toHaveAttribute("data-push", "preempt");
});
