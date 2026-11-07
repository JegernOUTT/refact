import { expect, test } from "@playwright/test";

const fixture = "/tests/e2e/route-showcase.html?route=workspace-chrome";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.clear());
});

test("rail and dock reach the shell top beside the content toolbar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(
    "/tests/e2e/route-showcase.html?route=chat-dnd&toolbar_alignment&unsplit",
  );
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const shell = page.getByTestId("workspace-shell");
  const rail = page.getByRole("navigation", { name: "Workspace sections" });
  const dock = page.getByTestId("workspace-dock");
  const toolbar = page.getByRole("toolbar").first();
  await expect(dock).toBeVisible();
  const [s, r, d, t] = await Promise.all(
    [shell, rail, dock, toolbar].map((element) => element.boundingBox()),
  );
  expect(r!.y).toBe(s!.y);
  expect(d!.y).toBe(s!.y);
  expect(t!.y).toBe(s!.y);
  expect(t!.x).toBeGreaterThanOrEqual(d!.x + d!.width - 1);
  await expect(page.getByRole("toolbar")).toHaveCount(1);
});

test("narrow entry closes once, manual reopen fills the viewport, widening stays closed", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(fixture);
  await expect(page.getByTestId("workspace-dock")).toBeVisible();
  await page.setViewportSize({ width: 600, height: 800 });
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const sheet = page.getByRole("dialog", { name: "Workspace dock" });
  await expect(sheet).toBeVisible();
  await expect.poll(async () => (await sheet.boundingBox())?.x).toBe(0);
  const box = (await sheet.boundingBox())!;
  expect(box).toMatchObject({ x: 0, y: 0, width: 600, height: 800 });
  const close = sheet.getByRole("button", { name: "Close workspace panel" });
  expect((await close.boundingBox())!.x).toBeGreaterThan(550);
  expect(
    await sheet.evaluate((el) => getComputedStyle(el).animationName),
  ).toContain("dock-sheet-in");
  await sheet.evaluate((el) => {
    el.addEventListener("animationstart", (event) => {
      document.documentElement.dataset.dockExitAnimation = (
        event as AnimationEvent
      ).animationName;
    });
  });
  await close.click();
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.dataset.dockExitAnimation),
    )
    .toContain("dock-sheet-out");
  await expect(sheet).toHaveCount(0);
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect(page.getByTestId("workspace-dock")).toHaveCount(0);
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await expect(page.getByTestId("workspace-dock")).toBeVisible();
});

test("initial narrow hydration stays closed and reduced-motion reopen supports Escape", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto(fixture);
  await expect(page.getByTestId("workspace-shell")).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const sheet = page.getByRole("dialog", { name: "Workspace dock" });
  await expect(sheet).toBeVisible();
  expect(await sheet.evaluate((el) => getComputedStyle(el).animationName)).toBe(
    "none",
  );
  expect((await sheet.boundingBox())!.width).toBe(360);
  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);
});
