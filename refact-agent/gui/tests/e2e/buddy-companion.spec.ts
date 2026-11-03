import { expect, test, type Page } from "@playwright/test";

test.use({ reducedMotion: "reduce" });

const shots = process.env.BUDDY_SHOTS_DIR ?? "test-results";

async function openFixture(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  await page.goto("/tests/e2e/buddy-companion.html");
  const rail = page.getByRole("button", { name: /^Buddy/ });
  await expect(rail).toHaveAccessibleName("Buddy, 4 need you");
  return rail;
}

test("rail icon toggles a non-modal popover with needs-you rows and the companion chat", async ({
  page,
}) => {
  const rail = await openFixture(page, 1100, 860);
  const nav = page.getByRole("navigation", { name: "Workspace sections" });
  const railBox = await rail.boundingBox();
  const navBox = await nav.boundingBox();
  expect(railBox!.y + railBox!.height).toBeGreaterThan(
    navBox!.y + navBox!.height - 60,
  );
  await expect(page.getByText("launcher")).toHaveCount(0);
  await page.screenshot({ path: `${shots}/buddy-1100-closed.png` });

  await rail.click();
  await expect(rail).toHaveAttribute("aria-pressed", "true");
  const dialog = page.getByRole("dialog", { name: "Buddy" });
  await expect(dialog).toHaveAttribute("aria-modal", "false");
  const box = await dialog.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(navBox!.x + navBox!.width);
  expect(Math.round(box!.y + box!.height)).toBeLessThanOrEqual(860 - 8);

  const composer = dialog.getByTestId("chat-form-textarea");
  await expect(composer).toBeVisible();
  await expect(composer).toBeFocused();

  const needs = dialog.getByRole("region", { name: "Needs you" });
  await expect(needs.getByRole("listitem")).toHaveCount(3);
  await needs.getByRole("button", { name: "Show all (4)" }).click();
  await expect(needs.getByRole("listitem")).toHaveCount(4);
  await expect(dialog.getByText(/waiting on a shell permission/)).toBeVisible();
  for (const control of ["model", "mode"]) {
    const host = dialog.locator(`[data-composer-control="${control}"]`);
    await expect(host).toHaveAttribute("data-hidden", "true");
    await expect(host).toBeHidden();
  }
  await expect(dialog).not.toContainText(/Approve once|Ask Buddy|coverage/);
  await page.screenshot({ path: `${shots}/buddy-1100-open.png` });

  const main = page.getByRole("textbox", { name: "Main source draft" });
  await main.click();
  await main.fill("Main draft remains editable");
  await expect(main).toBeFocused();
  await expect(main).toHaveValue("Main draft remains editable");
  await expect(dialog).toBeVisible();

  await composer.click();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(rail).toBeFocused();
  const posts = await page.evaluate(
    () => (window as unknown as { fixturePosts: string[] }).fixturePosts,
  );
  expect(
    posts.filter((path) => path.startsWith("/v1/buddy/companion")),
  ).toEqual(["/v1/buddy/companion/source-a"]);
  expect(posts.some((path) => path.includes("/approvals/"))).toBe(false);
});

test("sending from Buddy posts only a user message to the companion chat", async ({
  page,
}) => {
  const rail = await openFixture(page, 1100, 860);
  await rail.click();
  const composer = page
    .getByRole("dialog", { name: "Buddy" })
    .getByTestId("chat-form-textarea");
  await composer.fill("What is the source chat waiting on?");
  await composer.press("Enter");
  const companionCommands = () =>
    page.evaluate(() =>
      (
        window as unknown as {
          fixtureCommands: { chatId: string; type: unknown }[];
        }
      ).fixtureCommands
        .filter((command) => command.chatId === "buddy-fixture")
        .map((command) => command.type),
    );
  await expect.poll(companionCommands).toContain("user_message");
  expect(await companionCommands()).toEqual(["user_message"]);
});

test("needs-you rows open the source chat and keep the popover open", async ({
  page,
}) => {
  const rail = await openFixture(page, 1100, 860);
  await rail.click();
  const dialog = page.getByRole("dialog", { name: "Buddy" });
  const first = dialog
    .getByRole("region", { name: "Needs you" })
    .getByRole("button")
    .first();
  await expect(first).toContainText("Tool permission");
  await expect(first).toHaveAttribute("title", /requires your permission/);
  await first.click();
  await expect(dialog).toBeVisible();
});

test("header drag and corner resize move the popover, clamp it and persist geometry", async ({
  page,
}) => {
  const rail = await openFixture(page, 1100, 860);
  await rail.click();
  const dialog = page.getByRole("dialog", { name: "Buddy" });
  await expect(dialog.getByTestId("chat-form-textarea")).toBeFocused();
  const header = page.getByTestId("buddy-popover-header");
  const before = await dialog.boundingBox();
  const headerBox = await header.boundingBox();
  await page.mouse.move(
    headerBox!.x + 40,
    headerBox!.y + headerBox!.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    headerBox!.x + 240,
    headerBox!.y + headerBox!.height / 2 - 100,
    { steps: 5 },
  );
  await page.mouse.up();
  const moved = await dialog.boundingBox();
  expect(Math.round(moved!.x - before!.x)).toBe(200);
  expect(Math.round(moved!.y - before!.y)).toBe(-100);

  const handle = page.getByTestId("buddy-resize-handle");
  const handleBox = await handle.boundingBox();
  await page.mouse.move(handleBox!.x + 4, handleBox!.y + 4);
  await page.mouse.down();
  await page.mouse.move(handleBox!.x + 84, handleBox!.y + 44, { steps: 5 });
  await page.mouse.up();
  const resized = await dialog.boundingBox();
  expect(Math.round(resized!.width - moved!.width)).toBe(80);
  expect(Math.round(resized!.height - moved!.height)).toBe(40);
  await page.screenshot({ path: `${shots}/buddy-1100-dragged.png` });

  await page.mouse.move(headerBox!.x + 240, headerBox!.y + 10);
  await page.mouse.down();
  await page.mouse.move(5000, 5000, { steps: 3 });
  await page.mouse.up();
  const clamped = await dialog.boundingBox();
  expect(Math.round(clamped!.x + clamped!.width)).toBeLessThanOrEqual(1100);
  expect(Math.round(clamped!.y + clamped!.height)).toBeLessThanOrEqual(860);

  await page.getByRole("button", { name: "Minimize Buddy" }).click();
  await expect(dialog).toBeHidden();
  await page.reload();
  await page.getByRole("button", { name: /^Buddy/ }).click();
  const restored = await page
    .getByRole("dialog", { name: "Buddy" })
    .boundingBox();
  expect(Math.round(restored!.width)).toBe(Math.round(clamped!.width));
  expect(Math.round(restored!.x)).toBe(Math.round(clamped!.x));
});

test("narrow 480px viewport fills the space beside the rail and keeps the icon usable", async ({
  page,
}) => {
  const rail = await openFixture(page, 480, 900);
  await page.screenshot({ path: `${shots}/buddy-480-closed.png` });
  await rail.click();
  const dialog = page.getByRole("dialog", { name: "Buddy" });
  const box = await dialog.boundingBox();
  const navBox = await page
    .getByRole("navigation", { name: "Workspace sections" })
    .boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(navBox!.x + navBox!.width);
  expect(Math.round(box!.x + box!.width)).toBe(480 - 8);
  await expect(dialog.getByTestId("chat-form-textarea")).toBeVisible();
  const main = page.getByRole("textbox", { name: "Main source draft" });
  await main.fill("Still editable");
  await expect(main).toHaveValue("Still editable");
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({ path: `${shots}/buddy-480-open.png` });
  await rail.click();
  await expect(dialog).toBeHidden();
});
