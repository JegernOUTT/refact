import { expect, test, type Locator } from "@playwright/test";

const agenticRows = [
  ["compact-message", "Message", null],
  ["compact-plan", "Plan", null],
  ["compact-goal", "Goal", "No verdict"],
  ["compact-claim", "Claim card", "Failed"],
  ["compact-note", "Save task memory", null],
  ["compact-inspect", "Status", "Needs you"],
  ["compact-control", "Merge agent", null],
  ["launch-preview", "Launch", "Blocked"],
  ["compact-status", "Agents", null],
  ["compact-pulse", "Pulse", null],
  ["compact-diff", "Diff", null],
  ["compact-documents", "Documents", null],
  ["compact-document", "Document", null],
  ["compact-paused", "Goal", null],
  ["compact-board", "Task board", null],
] as const;

const otherRows = [
  "compact-final",
  "compact-unknown",
  "compact-background",
  "compact-review",
  "compact-legacy-launch",
  "compact-task-done",
  "compact-awaiting",
  "compact-question",
] as const;

function item(row: Locator) {
  return row.locator("[data-transcript-item]").first();
}

function toggle(row: Locator) {
  return item(row).getByTestId("transcript-toggle").first();
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

test("agent_message names the recipient and shows Sent without a Queued pill", async ({
  page,
}) => {
  await page.goto("/tests/e2e/compact-agentic.html");
  const row = page.getByTestId("compact-message");
  await expect(row.getByTestId("transcript-target")).toHaveText("bg-agent");
  await expect(row.getByTestId("transcript-meta")).toHaveText(
    "Keep the change scoped to the parser",
  );
  await expect(row.getByText("Queued", { exact: true })).toHaveCount(0);
  await toggle(row).click();
  await expect(row.getByTestId("transcript-facts")).toContainText("Sent");
  await expect(row.getByTestId("agentic-tool-message")).toHaveText(
    "Keep the change scoped to the parser",
  );
  const raw = row.getByTestId("transcript-raw");
  await raw.getByRole("button", { name: "Raw" }).click();
  await expect(raw).toContainText("Message queued.");
});

test("stored open choice survives unmount and remount", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/tests/e2e/compact-agentic.html");
  const plan = page.getByTestId("compact-plan");
  await toggle(plan).focus();
  await page.keyboard.press("Enter");
  await expect(toggle(plan)).toHaveAttribute("aria-expanded", "true");
  await page.getByRole("button", { name: "Unmount transcript" }).click();
  await page.getByRole("button", { name: "Mount transcript" }).click();
  await expect(toggle(plan)).toHaveAttribute("aria-expanded", "true");
  await expect(toggle(page.getByTestId("compact-goal"))).toHaveAttribute(
    "aria-expanded",
    "false",
  );
});

test("controls stay unreachable while collapsed and only send on explicit click", async ({
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
  const preview = page.getByTestId("launch-preview");
  await expect(toggle(preview)).toHaveAttribute("aria-expanded", "true");
  const nextSource = preview.getByRole("button", {
    name: "Next source",
    exact: true,
  });
  await nextSource.click();
  await expect(
    preview.getByText("completed-parent-message", { exact: true }),
  ).toBeVisible();
  await expect(nextSource).toBeDisabled();

  const pulse = page.getByTestId("compact-pulse");
  await expect(item(pulse)).toHaveAttribute("data-transcript-status", "done");
  await expect(pulse.getByTestId("transcript-status-pill")).toHaveCount(0);
  await expect(pulse.getByTestId("transcript-meta")).toHaveText("paused");
  const pause = pulse.getByRole("button", {
    name: "Pause",
    exact: true,
    includeHidden: true,
  });
  await expect(pause).not.toBeVisible();
  await toggle(pulse).focus();
  await page.keyboard.press("Tab");
  await expect(toggle(page.getByTestId("compact-diff"))).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(toggle(pulse)).toBeFocused();
  expect(commands).toEqual([]);
  await toggle(pulse).click();
  await pulse.getByRole("button", { name: "Pause", exact: true }).click();
  await expect
    .poll(() => commands)
    .toEqual([
      expect.objectContaining({
        type: "user_message",
        priority: true,
        content:
          'pause(target={"kind":"card","card_id":"T-14"}, reason="Paused from agent pulse view.")',
      }),
    ]);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Close", exact: true })
    .click();

  const document = page.getByTestId("compact-document");
  await toggle(document).click();
  await expect(
    document.getByText("Preserve the complete document body.", {
      exact: true,
    }),
  ).toBeVisible();
});

test("agentic transcript rows preserve dedicated summaries and disclosure", async ({
  page,
}) => {
  await page.goto("/tests/e2e/compact-agentic.html");
  for (const [id] of agenticRows)
    await expect(page.getByTestId(id).getByTestId("generic-tool")).toHaveCount(
      0,
    );
  for (const id of otherRows) await expect(page.getByTestId(id)).toBeVisible();
  for (const [id, verb, pill] of agenticRows) {
    const row = page.getByTestId(id);
    await expect(row.locator("[data-transcript-item]")).toHaveCount(1);
    await expect(row.getByTestId("transcript-verb")).toHaveText(verb);
    const status = row.getByTestId("transcript-status-pill");
    if (pill === null) await expect(status).toHaveCount(0);
    else await expect(status).toHaveText(pill);
    await expect(row.getByText("More details")).toHaveCount(0);
  }
  for (const [id] of agenticRows) {
    const row = page.getByTestId(id);
    if ((await toggle(row).getAttribute("aria-expanded")) === "false")
      await toggle(row).click();
    await expect(toggle(row)).toHaveAttribute("aria-expanded", "true");
    await expect(row.getByTestId("transcript-raw")).toHaveCount(1);
    await expect(row.locator("details")).toHaveCount(0);
    if (id === "compact-control")
      await expect(row.getByTestId("transcript-facts")).toContainText(
        "push and verification unconfirmed",
      );
    await toggle(row).click();
  }
});

for (const host of ["vscode", "jetbrains"]) {
  test(`${host} reduced-motion smoke renders the agentic rows`, async ({
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
    await expect(page.getByTestId("compact-awaiting")).toBeVisible();
    await toggle(page.getByTestId("compact-status")).click();
    await expect(toggle(page.getByTestId("compact-status"))).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });
}
