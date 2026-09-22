import { expect, test } from "@playwright/test";

const route = "/tests/e2e/route-showcase.html?route=chat-dnd&rebuild=1";

test("rebuilt disclosures, archive, legacy gate, cap reset and navigation", async ({
  page,
}, testInfo) => {
  test.setTimeout(60000);
  const escapedApi: string[] = [];
  await page.route("**/v1/**", async (request) => {
    escapedApi.push(request.request().url());
    await request.abort();
  });
  await page.goto(route);
  const reports = page.getByTestId("reconstructed-history-report");
  await expect(reports).toHaveCount(2);
  await expect(
    page.getByText("Original archive sentinel", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("After rebuild sentinel", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Latest rebuilt sentinel", { exact: true }),
  ).toBeHidden();
  await expect(
    page.getByText("Between reports sentinel", { exact: true }),
  ).toBeVisible();
  await reports.nth(1).locator("summary").click();
  await expect(
    page.getByText("Latest rebuilt sentinel", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Earlier rebuilt sentinel", { exact: true }),
  ).toBeHidden();
  await reports.nth(0).locator("summary").click();
  await expect(
    page.getByText("Earlier rebuilt sentinel", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Latest rebuilt sentinel", { exact: true }),
  ).toHaveCount(1);
  await testInfo.attach("two-reports-and-original-archive", {
    body: await page.locator("body").ariaSnapshot(),
    contentType: "text/plain",
  });
  await page.getByRole("tab", { name: "Idle Legacy archive fixture" }).click();
  await expect(page.getByRole("alert")).toContainText(
    "compressed with an older format",
  );
  await expect(
    page.getByText("Legacy original sentinel", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Rebuild context", exact: true })
    .click();
  await expect(
    page.getByRole("tab", { name: "LLM compression" }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("tab", { name: "Handoff" })).toHaveCount(0);
  await page.getByRole("tab", { name: "Compress in-place" }).click();
  await expect(
    page.getByText("Drop all context files", { exact: true }),
  ).toBeVisible();
  await testInfo.attach("legacy-gate-and-retained-tabs", {
    body: await page.locator("body").ariaSnapshot(),
    contentType: "text/plain",
  });
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Idle Rebuilt archive fixture" }).click();
  await page
    .getByRole("button", { name: /openai_codex_personal\/gpt-5\.5 · 128K/ })
    .click();
  await page.getByRole("button", { name: "Token limits", exact: true }).click();
  await expect(page.getByText(/New chats and Reset use 90%/)).toBeVisible();
  await page
    .getByRole("button", { name: "Reset auto-compression cap", exact: true })
    .click();
  await expect(
    page.getByRole("slider", { name: "Auto-compression cap", exact: true }),
  ).toHaveAttribute("aria-valuenow", "115200");
  await testInfo.attach("ninety-percent-reset", {
    body: await page.locator("body").ariaSnapshot(),
    contentType: "text/plain",
  });
  await page.keyboard.press("Escape");
  await page.getByRole("tab", { name: "Idle Legacy archive fixture" }).click();
  await expect(
    page.getByRole("button", { name: "New Chat", exact: true }),
  ).toBeEnabled();
  await page.reload();
  await expect(reports).toHaveCount(2);
  await page.getByRole("button", { name: "New Chat", exact: true }).click();
  await expect(
    page.getByRole("tab", { name: "Idle Rebuilt archive fixture" }),
  ).toBeVisible();
  await expect(
    page.getByRole("tab", { name: "Idle Legacy archive fixture" }),
  ).toBeVisible();
  const chatTabs = page
    .getByRole("tablist", { name: "Open workspace tabs" })
    .getByRole("tab");
  await expect(chatTabs).toHaveCount(3);
  await expect(chatTabs.last()).toHaveAttribute("aria-selected", "true");
  await expect(
    page.getByRole("combobox", { name: "Type @ or / for commands" }),
  ).toBeEmpty();
  await testInfo.attach("new-chat-after-reload", {
    body: await page.locator("body").ariaSnapshot(),
    contentType: "text/plain",
  });
  await testInfo.attach("new-chat-screenshot", {
    body: await page.screenshot(),
    contentType: "image/png",
  });
  expect(escapedApi).toEqual([]);
});
