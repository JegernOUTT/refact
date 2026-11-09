import { expect, test } from "@playwright/test";

const variants = [
  "exhausted",
  "waiting",
  "attempting",
  "needs_action",
  "cancelled",
  "succeeded",
  "privacy",
  "cooldown",
  "goal-held",
  "queued",
  "request",
  "request-no-retry",
  "multiple",
  "unbroken",
  "multiline",
];

for (const theme of ["dark", "light"]) {
  for (const width of [240, 360, 768, 1280]) {
    test(`error matrix: ${theme} at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 1100 });
      await page.goto("/tests/e2e/recovery-showcase.html");
      if (theme === "light")
        await page.getByRole("button", { name: "Toggle theme" }).click();
      for (const scenario of variants) {
        await page
          .getByRole("combobox", { name: "Scenario" })
          .selectOption(scenario);
        const standalone = scenario.startsWith("request");
        const panel = page.getByRole("region", {
          name: standalone ? "Request error" : "Recovery status",
          exact: true,
        });
        if (scenario === "succeeded") {
          await expect(panel).toHaveCount(0);
          continue;
        }
        await expect(panel).toHaveCount(1);
        await panel.scrollIntoViewIfNeeded();
        // The whitespace-only assistant must start with tools, not a padded Markdown box.
        const toolFooter = page.getByTestId("message-footer").last();
        await expect(toolFooter).toHaveAttribute("data-overlay", "true");
        expect(
          await toolFooter.evaluate((footer) => {
            const wrapper = footer.parentElement!;
            const row = wrapper.querySelector(
              '[data-testid="transcript-row"]',
            )!;
            return (
              row.getBoundingClientRect().top -
              wrapper.getBoundingClientRect().top
            );
          }),
        ).toBeLessThan(2);
        if (!standalone) {
          for (const closed of await panel
            .locator('[data-recovery-panel][aria-hidden="true"]')
            .all()) {
            await expect
              .poll(() =>
                closed.evaluate(
                  (element) => element.getBoundingClientRect().height,
                ),
              )
              .toBe(0);
          }
        }
        if (!standalone && scenario !== "unbroken") {
          const titleBox = await page
            .getByTestId("transcript-title")
            .last()
            .boundingBox();
          const alertBox = await panel.getByRole("alert").boundingBox();
          expect(titleBox).not.toBeNull();
          expect(Math.abs(alertBox!.x - titleBox!.x)).toBeLessThanOrEqual(1.1);
        }
        await expect
          .poll(() =>
            page.evaluate(
              () => document.documentElement.scrollWidth <= innerWidth,
            ),
          )
          .toBe(true);
        expect(
          await panel.evaluate(
            (element) => element.scrollWidth <= element.clientWidth,
          ),
        ).toBe(true);
        const buttons = panel.getByRole("button");
        const boxes = [];
        for (const button of await buttons.all()) {
          await expect(button).toHaveAccessibleName(/.+/);
          expect(await button.innerText()).toBe("");
          const box = await button.boundingBox();
          expect(box).not.toBeNull();
          expect(box!.width).toBeCloseTo(28, 0);
          expect(box!.height).toBeCloseTo(28, 0);
          boxes.push(box!);
        }
        if (boxes.length)
          expect(
            Math.max(...boxes.map((box) => box.y)) -
              Math.min(...boxes.map((box) => box.y)),
          ).toBeLessThan(width === 240 ? 33 : 1);
        if (scenario === "request-no-retry")
          await expect(buttons).toHaveCount(0);
        if (scenario === "privacy")
          await expect(panel.getByRole("alert")).toContainText(
            "No private content was sent",
          );
        if (scenario === "multiple")
          await expect(panel.getByRole("alert")).toContainText(
            "error sending request for url",
          );
        if (scenario === "cooldown") {
          await expect(panel.getByRole("status")).toContainText(
            "Provider cooldown",
          );
          await expect(
            panel.getByRole("button", { name: "Retry now", exact: true }),
          ).toBeDisabled();
        }
        if (scenario === "goal-held") {
          await expect(
            panel.getByRole("button", { name: "Resume goal", exact: true }),
          ).toBeVisible();
          await expect(
            panel.getByRole("button", { name: "Retry", exact: true }),
          ).toHaveCount(0);
        }
        if (scenario === "queued")
          await expect(
            panel.getByRole("button", {
              name: "Cancel queued retry",
              exact: true,
            }),
          ).toBeVisible();
        if (scenario === "unbroken" || scenario === "multiline") {
          const alert = panel.getByRole("alert");
          expect(
            await alert.evaluate(
              (element) => element.scrollWidth <= element.clientWidth,
            ),
          ).toBe(true);
          if (width <= 360)
            expect((await alert.boundingBox())!.height).toBeGreaterThan(40);
        }
      }
    });
  }
}

for (const theme of ["dark", "light"]) {
  for (const width of [240, 360, 768, 1280]) {
    for (const command of [
      "sending",
      "pending",
      "uncertain",
      "request-error",
      "stop-error",
      "resume-error",
      "cancel-error",
      "cancelling",
    ]) {
      test(`command ${command}: ${theme} at ${width}px`, async ({ page }) => {
        await page.setViewportSize({ width, height: 1100 });
        const scenario =
          command === "stop-error"
            ? "attempting"
            : command === "resume-error"
              ? "goal-held"
              : command.startsWith("cancel")
                ? "queued"
                : "exhausted";
        await page.goto(
          `/tests/e2e/recovery-showcase.html?command=${command}&scenario=${scenario}`,
        );
        if (theme === "light")
          await page.getByRole("button", { name: "Toggle theme" }).click();
        const panel = page.getByRole("region", { name: "Recovery status" });
        const retry = panel.getByRole("button", { name: "Retry", exact: true });
        const action =
          command === "stop-error"
            ? "Stop"
            : command === "resume-error"
              ? "Resume goal"
              : command.startsWith("cancel")
                ? "Cancel queued retry"
                : "Retry";
        await panel.getByRole("button", { name: action, exact: true }).click();
        if (command === "sending" || command === "pending") {
          await expect(panel.getByRole("status")).toContainText(
            command === "sending" ? "Submitting" : "Awaiting confirmation",
          );
          await expect(retry).toHaveCount(0);
          await expect(
            panel.getByRole("button", {
              name: /Submitting|Awaiting confirmation/,
            }),
          ).toBeDisabled();
        } else if (command === "uncertain") {
          await expect(panel.getByRole("status")).toContainText(
            "Command not confirmed",
          );
          await expect(retry).toHaveCount(0);
          await expect(
            panel.getByRole("button", { name: "Check status", exact: true }),
          ).toBeEnabled();
        } else if (command === "request-error") {
          await expect(
            panel.getByRole("alert").filter({ hasText: "Retry rejected" }),
          ).toBeVisible();
          await expect(retry).toBeEnabled();
        } else if (command === "cancelling") {
          await expect(panel.getByRole("status")).toContainText("Cancelling");
          await expect(
            panel.getByRole("button", { name: "Cancelling…", exact: true }),
          ).toBeDisabled();
        } else {
          const message =
            command === "stop-error"
              ? "Could not request Stop"
              : command === "resume-error"
                ? "Could not resume the goal"
                : "Could not cancel the queued retry";
          await expect(
            panel.getByRole("alert").filter({ hasText: message }),
          ).toBeVisible();
          await expect(
            panel.getByRole("button", { name: action, exact: true }),
          ).toBeEnabled();
        }
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true);
        expect(
          await panel.evaluate(
            (element) => element.scrollWidth <= element.clientWidth,
          ),
        ).toBe(true);
        for (const button of await panel
          .getByRole("group", { name: "Recovery actions", exact: true })
          .getByRole("button")
          .all()) {
          await expect(button).toHaveAccessibleName(/.+/);
          expect(await button.innerText()).toBe("");
          const box = await button.boundingBox();
          expect(box!.width).toBeCloseTo(28, 0);
          expect(box!.height).toBeCloseTo(28, 0);
        }
      });
    }
  }
}

test("compact recovery controls retain keyboard toggles and tooltip names", async ({
  page,
}) => {
  await page.goto("/tests/e2e/recovery-showcase.html?scenario=privacy");
  const panel = page.getByRole("region", { name: "Recovery status" });
  const details = panel.getByRole("button", { name: "Details", exact: true });
  await details.focus();
  await expect(details).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(details).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Space");
  await expect(details).toHaveAttribute("aria-expanded", "false");
  await details.hover();
  await expect(
    page.getByRole("tooltip", { name: "Details", exact: true }),
  ).toBeVisible();
});

test("recovery stays unified, toggles close, sends collapse and success disappears", async ({
  page,
}) => {
  await page.goto("/tests/e2e/recovery-showcase.html");
  const panel = page.getByRole("region", { name: "Recovery status" });
  await expect(panel).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(1);
  await expect(panel.getByRole("alert")).toContainText(
    "error sending request for url",
  );
  await expect(panel.getByRole("alert")).toContainText(
    "response header timeout after 30s",
  );
  await expect(panel.getByText("Request failed", { exact: true })).toHaveCount(
    0,
  );
  const retryBox = await panel
    .getByRole("button", { name: "Retry", exact: true })
    .boundingBox();
  const errorBox = await panel.getByRole("alert").boundingBox();
  expect(retryBox).not.toBeNull();
  expect(errorBox).not.toBeNull();
  expect(retryBox!.y).toBeGreaterThanOrEqual(errorBox!.y + errorBox!.height);
  await panel.screenshot({ path: "test-results/recovery-default-dark.png" });
  const details = panel.getByRole("button", { name: "Details", exact: true });
  const options = panel.getByRole("button", { name: "Options", exact: true });
  await details.click();
  await expect(details).toHaveAttribute("aria-expanded", "true");
  await options.click();
  await expect(details).toHaveAttribute("aria-expanded", "false");
  await expect(options).toHaveAttribute("aria-expanded", "true");
  await panel.screenshot({ path: "test-results/recovery-options-dark.png" });
  await options.click();
  await expect(options).toHaveAttribute("aria-expanded", "false");
  await options.click();
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Continue reviewing");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(options).toHaveAttribute("aria-expanded", "false");
  await page
    .getByRole("combobox", { name: "Scenario" })
    .selectOption("exhausted");
  await panel.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(panel.getByRole("status")).toContainText("Request in progress");
  await expect(
    panel.getByRole("button", { name: "Stop", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("combobox", { name: "Scenario" })
    .selectOption("succeeded");
  await expect(panel).toHaveCount(0);
  await expect(
    page.getByRole("region", { name: "Request error", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText(/Recovered|Previous recovery/)).toHaveCount(0);
});

test("recovery controls stay contained at narrow and wide widths", async ({
  page,
}) => {
  await page.goto("/tests/e2e/recovery-showcase.html");
  for (const theme of ["dark", "light"]) {
    if (theme === "light")
      await page.getByRole("button", { name: "Toggle theme" }).click();
    for (const width of [240, 360, 768, 1280]) {
      await page.setViewportSize({ width, height: 900 });
      const panel = page.getByRole("region", { name: "Recovery status" });
      const options = panel.getByRole("button", {
        name: "Options",
        exact: true,
      });
      await options.click();
      await expect(options).toHaveAttribute("aria-expanded", "true");
      await page.mouse.move(0, 0);
      const details = panel.getByRole("button", {
        name: "Details",
        exact: true,
      });
      await expect
        .poll(async () => {
          const boxes = await Promise.all([
            details.boundingBox(),
            options.boundingBox(),
          ]);
          expect(boxes[0]).not.toBeNull();
          expect(boxes[1]).not.toBeNull();
          return Math.abs((boxes[0]?.y ?? 0) - (boxes[1]?.y ?? 0));
        })
        .toBeLessThan(1);
      const retryBox = await panel
        .getByRole("button", { name: "Retry", exact: true })
        .boundingBox();
      const alertBox = await panel.getByRole("alert").boundingBox();
      expect(retryBox).not.toBeNull();
      expect(Math.abs(retryBox!.x - alertBox!.x)).toBeLessThan(1);
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
      if (width >= 768) {
        const fields = await panel.getByRole("spinbutton").all();
        const boxes = await Promise.all(
          fields.map((field) => field.boundingBox()),
        );
        expect(boxes).toHaveLength(3);
        expect(Math.abs((boxes[0]?.y ?? 0) - (boxes[1]?.y ?? 0))).toBeLessThan(
          1,
        );
        expect(Math.abs((boxes[1]?.y ?? 0) - (boxes[2]?.y ?? 0))).toBeLessThan(
          1,
        );
      }
      await options.click();
      await expect(options).toHaveAttribute("aria-expanded", "false");
    }
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  const details = page.getByRole("button", { name: "Details", exact: true });
  await details.click();
  const id = await details.getAttribute("aria-controls");
  expect(id).toBeTruthy();
  expect(
    await page
      .locator(`[id="${id}"]`)
      .evaluate((element) => getComputedStyle(element).transitionDuration),
  ).toBe("0s");
  await page
    .getByRole("region", { name: "Recovery status" })
    .screenshot({ path: "test-results/recovery-details-light.png" });
});
