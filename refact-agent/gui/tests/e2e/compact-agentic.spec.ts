import { expect, test, type Locator, type Page } from "@playwright/test";

const rows = [
  ["compact-message", "Queued"],
  ["compact-plan", "Plan accepted"],
  ["compact-goal", "Validation unavailable"],
  ["compact-claim", "Request failed"],
  ["compact-note", "Outcome unconfirmed"],
  ["compact-inspect", "Approval needed"],
  ["compact-control", "Merged locally"],
  ["compact-final", "Reported success"],
  ["compact-unknown", "Outcome unconfirmed"],
  ["compact-background", "Running"],
  ["launch-preview", "Launch blocked"],
  ["compact-status", "Snapshot received"],
  ["compact-pulse", "Agent paused"],
  ["compact-diff", "Diff received"],
  ["compact-documents", "Response received"],
  ["compact-document", "Response received"],
  ["compact-paused", "Paused (snapshot)"],
  ["compact-board", "Outcome unconfirmed"],
  ["compact-review", "Review unavailable"],
  ["compact-legacy-launch", "Report unverified"],
  ["compact-task-done", "Report unverified"],
  ["compact-awaiting", "Awaiting report"],
  ["compact-question", "Needs answer"],
] as const;

function outer(row: Locator) {
  return row.locator('[data-density="compact"]').first();
}

function toggle(row: Locator) {
  return outer(row).locator("button[aria-expanded]").first();
}

async function expectCollapsedAction(action: Locator) {
  for (const button of await action.all()) {
    await expect(button).toHaveAttribute("tabindex", "-1");
    await expect
      .poll(() =>
        button.evaluate((element) => {
          const body = element.closest('.rf-expand-grid[data-open="false"]');
          const shell = body?.firstElementChild;
          return {
            inert: body?.hasAttribute("inert"),
            ownerClosed: body?.parentElement?.matches(
              'section[data-open="false"]',
            ),
            controlled:
              body?.parentElement
                ?.querySelector('button[aria-expanded="false"]')
                ?.getAttribute("aria-controls") === body?.id,
            height: body?.getBoundingClientRect().height,
            shellHeight: shell?.getBoundingClientRect().height,
            overflow: shell ? getComputedStyle(shell).overflow : null,
            opacity: shell ? getComputedStyle(shell).opacity : null,
            pointerEvents: body ? getComputedStyle(body).pointerEvents : null,
          };
        }),
      )
      .toEqual({
        inert: true,
        ownerClosed: true,
        controlled: true,
        height: 0,
        shellHeight: 0,
        overflow: "hidden",
        opacity: "0",
        pointerEvents: "none",
      });
  }
}

async function expectExpandedAction(action: Locator) {
  await expect(action).toBeVisible();
  await expect(action).toBeEnabled();
  await expect(action).not.toHaveAttribute("tabindex", "-1");
  await expect
    .poll(() =>
      action.evaluate((element) => {
        const body = element.closest(".rf-expand-grid");
        const shell = body?.firstElementChild;
        return (
          body !== null &&
          shell !== null &&
          shell !== undefined &&
          body.getAttribute("data-open") === "true" &&
          !element.closest("[inert]") &&
          body.getBoundingClientRect().height > 0 &&
          getComputedStyle(body).pointerEvents === "auto" &&
          shell.getBoundingClientRect().height > 0 &&
          getComputedStyle(shell).opacity === "1"
        );
      }),
    )
    .toBe(true);
}

async function geometry(page: Page) {
  const overflow = await page.getByRole("main").evaluate((main) => {
    const visible = (element: Element) =>
      element.getBoundingClientRect().height > 0 &&
      !element.closest('[inert], [aria-hidden="true"]');
    return {
      page: document.documentElement.scrollWidth > innerWidth + 1,
      rows: Array.from(main.querySelectorAll("article[data-testid]"))
        .filter((row) => {
          const rect = row.getBoundingClientRect();
          return (
            rect.left < -1 ||
            rect.right > innerWidth + 1 ||
            row.scrollWidth > row.clientWidth + 1
          );
        })
        .map((row) => row.getAttribute("data-testid")),
      details: Array.from(main.querySelectorAll("dd, nav, [role=radiogroup]"))
        .filter(visible)
        .filter((element) => element.scrollWidth > element.clientWidth + 1)
        .map((element) => element.textContent?.slice(0, 100)),
    };
  });
  expect(overflow).toEqual({ page: false, rows: [], details: [] });
}

test.beforeEach(async ({ page }) => {
  await page.route("**/v1/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "{}",
      headers: { "Access-Control-Allow-Origin": "*" },
    }),
  );
});

for (const width of [240, 360, 768, 1280]) {
  for (const theme of ["light", "dark"]) {
    test(`composed compact transcript at ${width}px ${theme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`/tests/e2e/compact-agentic.html?theme=${theme}`);
      await expect(
        page.locator(`[data-host="web"][data-appearance="${theme}"]`),
      ).toBeVisible();
      await expect(page.getByTestId("generic-tool")).toHaveCount(0);
      for (const [id, state] of rows) {
        const row = page.getByTestId(id);
        const card = outer(row);
        await expect(toggle(row)).toHaveAttribute("aria-expanded", "false");
        const height = await card.evaluate(
          (element) => element.getBoundingClientRect().height,
        );
        expect(Math.abs(height - 32), id).toBeLessThanOrEqual(1);
        const status = row.getByText(state, { exact: true }).first();
        await expect(status).toBeVisible();
        expect(
          await status.evaluate((element) => {
            const range = document.createRange();
            range.selectNodeContents(element);
            const text = range.getBoundingClientRect();
            const box = element.getBoundingClientRect();
            const card = element
              .closest('[data-density="compact"]')
              ?.getBoundingClientRect();
            return (
              text.left >= box.left - 1 &&
              text.right <= box.right + 1 &&
              box.left >= -1 &&
              card !== undefined &&
              text.left >= card.left - 1 &&
              text.right <= card.right + 1 &&
              box.right <= innerWidth + 1
            );
          }),
          `${id}: essential state must not be clipped`,
        ).toBe(true);
      }
      await geometry(page);
      for (const [id] of rows) {
        const row = page.getByTestId(id);
        await toggle(row).click();
        expect(
          await row
            .locator("dt")
            .evaluateAll(
              (facts) =>
                facts.filter((fact) => !fact.closest("[inert]")).length,
            ),
          `${id}: first facts`,
        ).toBeLessThanOrEqual(3);
        if (id !== "compact-question") {
          for (const action of [
            "Pause",
            "Steer",
            "Cancel",
            "Copy report",
            "Next source",
          ])
            await expectCollapsedAction(
              row.getByRole("button", {
                name: action,
                exact: true,
                includeHidden: true,
              }),
            );
        }
        if (id === "compact-control")
          await expect(row.locator("dd").first()).toContainText(
            "push and verification unconfirmed",
          );
        if (id === "launch-preview")
          await expect(
            row.getByRole("button", {
              name: "Next source",
              exact: true,
              includeHidden: true,
            }),
          ).toHaveCount(1);
        if (id === "compact-awaiting")
          await expect(outer(row)).toHaveAttribute("data-status", "idle");
        await geometry(page);
        await toggle(row).click();
      }
    });
  }
}

test("production nested keyboard disclosure and stored choice survive updates and remount", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/tests/e2e/compact-agentic.html");
  const background = page.getByTestId("compact-background");
  const trigger = toggle(background);
  await trigger.focus();
  await page.keyboard.press("Enter");
  const more = background.getByRole("button", { name: "More details" });
  await page.keyboard.press("Tab");
  await expect(
    background.getByRole("button", { name: "Open trajectory", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(more).toBeFocused();
  await page.keyboard.press("Space");
  await expect(more).toHaveAttribute("aria-expanded", "true");
  await expect(
    background.getByTestId("background-agent-expanded-detail"),
  ).toBeVisible();
  await page.getByRole("button", { name: "Receive child update" }).click();
  await expect(
    background.getByTestId("background-agent-inline-status"),
  ).toHaveText("Completed");
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  await expect(more).toHaveAttribute("aria-expanded", "true");
  await page.getByRole("button", { name: "Unmount transcript" }).click();
  await page.getByRole("button", { name: "Mount transcript" }).click();
  await expect(trigger).toHaveAttribute("aria-expanded", "true");
  await expect(more).toHaveAttribute("aria-expanded", "false");
  await expect(toggle(page.getByTestId("compact-plan"))).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  await trigger.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Tab");
  await expect(toggle(page.getByTestId("launch-preview"))).toBeFocused();
});

test("complete receipts and mutation intents remain behind explicit production disclosures", async ({
  page,
}) => {
  const commands: unknown[] = [];
  await page.route("**/v1/chats/*/commands", async (route) => {
    if (route.request().method() === "POST")
      commands.push(route.request().postDataJSON());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "{}",
      headers: { "Access-Control-Allow-Origin": "*" },
    });
  });
  await page.goto("/tests/e2e/compact-agentic.html");
  for (const [id, content] of [
    ["compact-message", "Message queued."],
    ["compact-control", '"merged":true'],
    ["compact-unknown", "Authority outcome was not recorded"],
    ["compact-review", "Review stage unavailable"],
    ["compact-final", "No verification evidence"],
    ["compact-task-done", "Implementation claimed; no checks supplied."],
    ["compact-legacy-launch", "Legacy child report; lifecycle unavailable"],
    ["compact-document", "Preserve the complete document body."],
    ["launch-preview", "No child spawned"],
  ]) {
    const row = page.getByTestId(id);
    await toggle(row).click();
    if (id === "launch-preview") {
      const more = row.getByRole("button", {
        name: "More details",
        exact: true,
      });
      await more.focus();
      await page.keyboard.press("Tab");
      await expect(toggle(page.getByTestId("compact-status"))).toBeFocused();
      await page.keyboard.press("Shift+Tab");
      await expect(more).toBeFocused();
    }
    await row
      .getByRole("button", { name: "More details", exact: true })
      .click();
    await expect(row).toContainText(content);
    await geometry(page);
  }
  await expect(
    page
      .getByTestId("compact-message")
      .getByLabel("Full response content", { exact: true }),
  ).toHaveText("Message queued.");
  await expect(
    page
      .getByTestId("compact-control")
      .getByLabel("Full response content", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByTestId("compact-final").getByTestId("final-report-view"),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("compact-document")
      .getByText("Preserve the complete document body.", { exact: true }),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("launch-preview")
      .getByLabel("Child launch preview", { exact: true }),
  ).toBeVisible();
  const preview = page.getByTestId("launch-preview");
  const nextSource = preview.getByRole("button", {
    name: "Next source",
    exact: true,
  });
  await expectExpandedAction(nextSource);
  await nextSource.focus();
  await expect(nextSource).toBeFocused();
  await nextSource.click();
  await expect(
    preview.getByText("completed-parent-message", { exact: true }),
  ).toBeVisible();
  await expect(nextSource).toBeDisabled();
  expect(commands).toEqual([]);
  const pulse = page.getByTestId("compact-pulse");
  await toggle(pulse).click();
  const pause = pulse.getByRole("button", {
    name: "Pause",
    exact: true,
    includeHidden: true,
  });
  await expect(pause).toHaveCount(1);
  await expectCollapsedAction(pause);
  const more = pulse.getByRole("button", {
    name: "More details",
    exact: true,
  });
  await more.focus();
  await page.keyboard.press("Tab");
  await expect(toggle(page.getByTestId("compact-diff"))).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(more).toBeFocused();
  await pulse
    .getByRole("button", { name: "More details", exact: true })
    .click();
  await expectExpandedAction(pause);
  await pause.focus();
  await expect(pause).toBeFocused();
  await pulse.getByRole("button", { name: "Pause", exact: true }).click();
  await expect
    .poll(() => commands)
    .toEqual([
      expect.objectContaining({
        type: "user_message",
        priority: true,
        content: expect.stringMatching(
          /^pause_agent\(card_id="T-14", reason=/u,
        ),
      }),
    ]);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Close", exact: true })
    .click();
  const question = page.getByTestId("compact-question");
  await toggle(question).click();
  await expect(
    question.getByRole("button", { name: "More details" }),
  ).toHaveCount(0);
  await question
    .getByRole("radio", { name: "Parser only", exact: true })
    .check();
  expect(commands).toHaveLength(1);
  await expect(
    question.getByRole("button", { name: "Submit response" }),
  ).toBeEnabled();
  await expect(question).toContainText("Needs answer");
});

for (const host of ["vscode", "jetbrains"]) {
  test(`${host} reduced-motion smoke keeps absent reports active only while busy`, async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.setViewportSize({ width: 360, height: 900 });
    await page.goto(
      `/tests/e2e/compact-agentic.html?host=${host}&theme=dark&busy`,
    );
    await expect(
      page.locator(`[data-host="${host}"][data-appearance="dark"]`),
    ).toBeVisible();
    const report = page.getByTestId("compact-awaiting");
    await expect(outer(report)).toHaveAttribute("data-status", "running");
    await expect(toggle(report)).toContainText("Awaiting report");
    await toggle(report).click();
    await expect(
      report.getByText("No report received", { exact: true }),
    ).toBeVisible();
    await geometry(page);
  });
}
