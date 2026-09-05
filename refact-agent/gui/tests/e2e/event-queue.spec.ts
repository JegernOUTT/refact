import { expect, test, type Locator, type Page } from "@playwright/test";
const route = "/tests/e2e/route-showcase.html?route=queue";
async function chooseTiming(page: Page, item: Locator, label: string) {
  const select = item.getByRole("combobox");
  if (await select.isVisible()) {
    await select.click();
    await page.getByRole("option", { name: label, exact: true }).click();
  } else await item.getByText(label, { exact: true }).first().click();
}
for (const width of [360, 1280]) {
  test(`queue controls and mock boundaries at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(route);
    await page.getByRole("button", { name: "Show all 5 queued items" }).click();
    const agent = page.locator('[data-delivery-id="delivery-agent-1"]');
    await expect(agent).toHaveAttribute("data-push", "append");
    const geometry = await page
      .getByTestId("queued-item")
      .evaluateAll((elements) =>
        elements.map((element) => {
          const r = element.getBoundingClientRect();
          return {
            left: r.left,
            right: r.right,
            width: r.width,
            overflow: element.scrollWidth > element.clientWidth,
          };
        }),
      );
    for (const item of geometry) {
      expect(item.left).toBeGreaterThanOrEqual(0);
      expect(item.right).toBeLessThanOrEqual(width);
      expect(item.overflow).toBe(false);
    }
    await page.screenshot({
      path: testInfo.outputPath(`queue-${width}.png`),
      fullPage: true,
    });
    await chooseTiming(page, agent, "When idle");
    await expect(agent).toHaveAttribute("data-push", "when_idle");
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
    await expect(
      page.locator('[data-delivery-id="delivery-process-1"]'),
    ).toHaveCount(0);
    await expect(cron).toHaveCount(1);
    await page.getByRole("button", { name: "Mock: final idle" }).click();
    await expect(page.getByTestId("queued-item")).toHaveCount(0);
    await page.getByText("Event history", { exact: true }).click();
    await expect(
      page
        .getByTestId("event-log")
        .getByText(
          "Active step cancelled; partial assistant and tool output discarded.",
        ),
    ).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}
test("cancel pending delivery and preserve legacy controls", async ({
  page,
}) => {
  await page.goto(route);
  await page.getByRole("button", { name: "Show all 5 queued items" }).click();
  const agent = page.locator('[data-delivery-id="delivery-agent-1"]');
  await agent.getByRole("button", { name: /Cancel pending delivery/ }).click();
  await expect(agent).toHaveCount(0);
  const user = page.locator('[data-delivery-id="queued-user-1"]');
  await user.getByRole("button", { name: "Change to send next" }).click();
  await expect(user).toHaveAttribute("data-push", "preempt");
  await user
    .getByRole("button", { name: "Click to edit queued message" })
    .click();
  await expect(user).toHaveCount(0);
});

for (const theme of ["light", "dark"]) {
  for (const width of [240, 360, 768, 1280]) {
    test(`expanded history and queue ${theme} ${width} reduced motion`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: 1000 });
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto(`${route}&theme=${theme}`);
      await page
        .getByRole("button", { name: "Show all 5 queued items" })
        .click();
      const history = page.getByTestId("event-log");
      await history.locator("summary").click();
      await expect(history.getByTestId("event-log-entry")).toHaveCount(15);
      const bounds = await page
        .getByTestId("queued-item")
        .evaluateAll((elements) =>
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
      const agent = page.locator('[data-delivery-id="delivery-agent-1"]');
      const select = agent.getByRole("combobox");
      if (await select.isVisible()) {
        await select.focus();
        await page.keyboard.press("Space");
        await page
          .getByRole("option", { name: "When idle", exact: true })
          .click();
        await expect(select).toBeFocused();
      } else {
        const radio = agent.getByRole("radio", { name: "When idle" });
        await radio.focus();
        await page.keyboard.press("Space");
        await expect(radio).toBeFocused();
      }
      await expect(agent).toHaveAttribute("data-push", "when_idle");
      await page.screenshot({
        path: testInfo.outputPath(`expanded-${theme}-${width}.png`),
        fullPage: true,
      });
    });
  }
}

for (const theme of ["light", "dark"]) {
  for (const width of [240, 1280]) {
    test(`100 long deliveries remain bounded ${theme} ${width}`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize({ width, height: 1000 });
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(`${route}&queue_count=100&theme=${theme}`);
      const disclosure = page.getByRole("button", {
        name: "Show all 100 queued items",
      });
      await expect(disclosure).toHaveAttribute("aria-expanded", "false");
      await disclosure.click();
      await expect(
        page.getByRole("button", { name: "Show fewer queued items" }),
      ).toHaveAttribute("aria-expanded", "true");
      await expect(page.getByTestId("queued-item")).toHaveCount(100);
      const bounds = await page
        .getByTestId("queued-item")
        .evaluateAll((elements) =>
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
      for (const id of [
        "queue-header",
        "chat-form-textarea",
        "chat-virtualized-list-wrapper",
      ]) {
        const area = page.getByTestId(id);
        await expect(area).toBeInViewport();
        const box = await area.boundingBox();
        expect(box!.height).toBeGreaterThan(0);
        expect(box!.y).toBeGreaterThanOrEqual(0);
        expect(box!.y + box!.height).toBeLessThanOrEqual(1000);
      }
      const last = page.locator('[data-delivery-id="delivery-stress-100"]');
      const stop = page.getByRole("button", {
        name: "Stop generation",
        exact: true,
      });
      await expect(stop).toBeInViewport();
      expect(
        await stop.evaluate((element) => {
          const rect = element.getBoundingClientRect();
          return [rect.left + 2, rect.right - 2].every((x) =>
            element.contains(
              document.elementFromPoint(x, rect.top + rect.height / 2),
            ),
          );
        }),
      ).toBe(true);
      await last.scrollIntoViewIfNeeded();
      await expect(last).toBeInViewport();
      expect(
        await last.evaluate((element) => element.parentElement!.scrollTop),
      ).toBeGreaterThan(0);
      await chooseTiming(page, last, "When idle");
      await expect(last).toHaveAttribute("data-push", "when_idle");
      await last
        .getByRole("button", { name: /Cancel pending delivery/ })
        .click();
      await expect(last).toHaveCount(0);
      await expect(page.getByTestId("queued-item")).toHaveCount(99);
      await page.screenshot({
        path: testInfo.outputPath(`stress-${theme}-${width}.png`),
        fullPage: true,
      });
      expect(errors).toEqual([]);
    });
  }
}

for (const width of [240, 1280]) {
  test(`HTTP failures retain timing and keyboard access ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${route}&queue_error=1`);
    await page.getByRole("button", { name: "Show all 5 queued items" }).click();
    const item = page.locator('[data-delivery-id="delivery-agent-1"]');
    await chooseTiming(page, item, "When idle");
    await expect(item.getByRole("alert")).toHaveText(
      "Could not change delivery timing. Try choosing it again.",
    );
    await expect(item).toHaveAttribute("data-push", "append");
    if (width === 240)
      await expect(item.getByRole("combobox")).toHaveText("After step");
    else
      await expect(
        item.getByRole("radio", { name: "After current step" }),
      ).toBeChecked();
    const cancel = item.getByRole("button", {
      name: /Cancel pending delivery/,
    });
    await cancel.focus();
    await page.keyboard.press("Enter");
    await expect(item.getByRole("alert")).toHaveText(
      "Could not update this queued item. Try the control again.",
    );
    await expect(item).toHaveCount(1);
    await expect(cancel).toBeEnabled();
    await cancel.focus();
    await expect(cancel).toBeFocused();
    await page.keyboard.press("Tab");
    expect(await page.evaluate(() => document.activeElement?.tagName)).not.toBe(
      "BODY",
    );
  });

  test(`pending timing suppresses duplicate requests ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`${route}&queue_pending=1`);
    await page.getByRole("button", { name: "Show all 5 queued items" }).click();
    const item = page.locator('[data-delivery-id="delivery-agent-1"]');
    await chooseTiming(page, item, "When idle");
    await expect(item).toHaveAttribute("aria-busy", "true");
    await expect(
      item.getByRole("button", { name: /Cancel pending delivery/ }),
    ).toBeDisabled();
    await chooseTiming(page, item, "Interrupt now");
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

test("entry motion follows the actual reduced-motion media preference", async ({
  page,
}) => {
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
