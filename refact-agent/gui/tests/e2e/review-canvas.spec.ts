import { expect, test } from "@playwright/test";

test("native explanation steps, pauses, replays and retains state on navigation", async ({
  page,
}) => {
  await page.clock.install();
  await page.goto("/tests/e2e/review-canvas.html?tour=rich&dynamic=evaluation");
  const visual = page.locator('[data-block-id="visual"]');
  await visual.scrollIntoViewIfNeeded();
  const frame = page.frameLocator('iframe[title="Tour visual"]');
  const status = frame.getByRole("status");
  await expect(status).toContainText("1. Caller");
  await frame.getByRole("button", { name: "Next step" }).focus();
  await page.keyboard.press("Enter");
  await expect(status).toContainText("2. Parser splits");
  await page.clock.pauseAt(new Date());
  await frame.getByRole("button", { name: "Play", exact: true }).focus();
  await page.keyboard.press("Space");
  await page.clock.runFor(900);
  await expect(status).toContainText("3. Each part is trimmed");
  await frame.getByRole("button", { name: "Pause", exact: true }).focus();
  await page.keyboard.press("Enter");
  await page.clock.runFor(1100);
  await expect(status).toContainText("3. Each part is trimmed");
  await page.getByRole("button", { name: "Sources & Evidence" }).click();
  await expect(status).toContainText("3. Each part is trimmed");
  await frame.getByRole("button", { name: "Replay", exact: true }).focus();
  await page.keyboard.press("Enter");
  await expect(status).toContainText("1. Caller");
  await visual.getByRole("button", { name: "Show static explanation" }).focus();
  await page.keyboard.press("Enter");
  await expect(visual.locator("iframe")).toHaveCount(0);
  await expect(
    visual.getByRole("region", { name: "Static explanation" }),
  ).toContainText("Trimming occurs before conversion");
  await visual
    .getByRole("button", { name: "Show live explanation" })
    .press("Enter");
  await expect(status).toContainText("1. Caller");
});

test("reduced motion defaults to readable static without starting iframe scripts", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/tests/e2e/review-canvas.html?tour=rich&dynamic=evaluation");
  const visual = page.locator('[data-block-id="visual"]');
  await visual.scrollIntoViewIfNeeded();
  await expect(
    visual.getByRole("region", { name: "Static explanation" }),
  ).toContainText("caller receives the mapped array");
  await expect(visual.locator("iframe")).toHaveCount(0);
  await visual
    .getByRole("button", { name: "Show live explanation" })
    .press("Enter");
  const frame = page.frameLocator('iframe[title="Tour visual"]');
  await expect(frame.getByRole("status")).toContainText("1. Caller");
  await frame.getByRole("button", { name: "Next step" }).press("Enter");
  await expect(frame.getByRole("status")).toContainText("2. Parser splits");
  await frame
    .getByRole("button", { name: "Show parser source" })
    .press("Enter");
  await expect(
    page.getByRole("radio", { name: "Review", exact: true }),
  ).toBeChecked();
});
