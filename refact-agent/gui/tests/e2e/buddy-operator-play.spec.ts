import { expect, test } from "@playwright/test";

for (const theme of ["dark", "light"]) {
  for (const width of [240, 360, 768, 1280]) {
    for (const reducedMotion of ["reduce", "no-preference"] as const) {
      test(`fixture play ${theme} ${width} ${reducedMotion}`, async ({
        page,
      }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.emulateMedia({ reducedMotion });
        await page.goto(
          `/tests/e2e/buddy-operator-preview.html?play=1&theme=${theme}`,
        );
        const preview = page.getByTestId("buddy-operator-play-preview");
        await preview
          .getByRole("button", { name: "Start fetch", exact: true })
          .click();
        await expect(preview).toBeFocused();
        await preview.getByRole("button", { name: /Throw left/ }).focus();
        await page.keyboard.press("1");
        await expect(preview).toHaveAttribute("data-play", "fetch:throwing");
        await expect(preview).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(preview).toHaveAttribute("data-suspended", "true");
        await preview
          .getByRole("button", { name: "Resume play", exact: true })
          .click();
        await expect(preview).toBeFocused();
        await page.keyboard.press("Escape");
        await expect(preview).toHaveAttribute("data-suspended", "true");
        await page.keyboard.press("Enter");
        await expect(preview).toHaveAttribute("data-suspended", "false");
        await preview
          .getByRole("button", { name: "Stop play", exact: true })
          .click();
        await expect(
          preview.getByRole("button", { name: "Start fetch", exact: true }),
        ).toBeFocused();
        await expect(preview).toHaveAttribute(
          "data-reduced-motion",
          String(reducedMotion === "reduce"),
        );
        expect(
          await preview.evaluate(
            (element) => element.scrollWidth <= element.clientWidth + 1,
          ),
        ).toBe(true);
        const world = page.getByTestId("buddy-world");
        await world
          .getByRole("button", { name: "Play fetch with Pixel" })
          .click();
        await world.getByRole("button", { name: "1: Throw left" }).click();
        await expect(world).toHaveAttribute("data-play", "fetch:throwing");
        await world
          .getByRole("button", { name: "Pause play", exact: true })
          .click();
        await expect(world.getByRole("status")).toContainText("paused");
        await world
          .getByRole("button", { name: "Stop play", exact: true })
          .click();
        await expect(
          world.getByRole("button", { name: "Play fetch with Pixel" }),
        ).toBeFocused();
      });
    }
  }
}

test("polish defaults off and text-only preserves play controls", async ({
  page,
}) => {
  await page.goto("/tests/e2e/buddy-operator-preview.html");
  await expect(page.getByTestId("buddy-world")).toHaveCount(0);
  await expect(page.getByTestId("buddy-operator-play-preview")).toHaveCount(0);
  await page.goto(
    "/tests/e2e/buddy-operator-preview.html?play=1&textonly=1&lowpower=1",
  );
  const world = page.getByTestId("buddy-world");
  await expect(world.getByTestId("buddy-world-character")).toHaveCount(0);
  await world.getByRole("button", { name: "Play fetch with Pixel" }).click();
  await world.getByRole("button", { name: "2: Throw ahead" }).click();
  await expect(world.getByRole("status")).toContainText("throwing");
});
