import { expect, test } from "@playwright/test";

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
  expect(Math.abs((retryBox?.y ?? 0) - (errorBox?.y ?? 0))).toBeLessThan(2);
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
      const detailsBox = await details.boundingBox();
      const panelBox = await panel.boundingBox();
      expect(panelBox).not.toBeNull();
      expect((detailsBox?.x ?? 0) - (panelBox?.x ?? 0)).toBeLessThan(20);
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
