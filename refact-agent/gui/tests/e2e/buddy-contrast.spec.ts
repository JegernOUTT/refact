import { expect, test } from "@playwright/test";

const BUDDY_ROUTE = "/tests/e2e/route-showcase.html?route=buddy&scene_speech=1";

test.describe("Buddy scene labels", () => {
  test("focusing a scene hotspot reveals its tooltip in light", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(`${BUDDY_ROUTE}&theme=light`);
    await page.getByTestId("buddy-home-content").waitFor();
    await page.getByRole("tab", { name: "Play", exact: true }).click();
    await expect(page.getByTestId("buddy-speech-bubble")).toBeVisible();

    const hotspot = page
      .getByTestId("buddy-world")
      .locator("button[aria-label]")
      .first();
    await hotspot.focus();
    await expect(hotspot).toBeFocused();
    await expect(page.getByRole("tooltip")).toBeVisible();
  });
});
