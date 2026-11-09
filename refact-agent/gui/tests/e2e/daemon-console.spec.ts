import { expect, test } from "@playwright/test";

test("workspace rail returns to the owning daemon without changing Home", async ({
  page,
}) => {
  await page.goto("/tests/e2e/daemon-console.html?theme=dark&rail=true");
  const link = page.getByRole("link", { name: "Daemon console", exact: true });
  await expect(link).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Home", exact: true }),
  ).toBeVisible();
  const target = new URL("/", page.url()).href;
  await expect(link).toHaveAttribute("href", target);
  await page.route(target, (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<!doctype html><h1>Daemon console landing</h1>",
    }),
  );
  await link.click();
  await expect(page).toHaveURL(target);
  await expect(
    page.getByRole("heading", { name: "Daemon console landing" }),
  ).toBeVisible();
});

test("selects real stream targets and keeps attached logs explicitly unavailable", async ({
  page,
}) => {
  await page.goto("/tests/e2e/daemon-console.html?theme=dark&page=activity");
  const output = page.locator("pre");
  await expect(output).toContainText("fixture heartbeat 1");
  await page.getByRole("combobox", { name: "Choose log source" }).click();
  await page.getByRole("option", { name: "refact", exact: true }).click();
  await expect(
    page.getByRole("combobox", { name: "Choose log source" }),
  ).toContainText("refact");
  await expect(output).toContainText("fixture heartbeat 1");
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const paused = await output.textContent();
  const emitted = await page.evaluate(() => window.daemonFixtureLogLines);
  const nextHeartbeat =
    Number(
      paused
        ?.match(/fixture heartbeat (\d+)/g)
        ?.at(-1)
        ?.match(/\d+/)?.[0] ?? 0,
    ) + 1;
  await page.waitForFunction(
    (previous) => window.daemonFixtureLogLines > previous,
    emitted,
  );
  await expect(output).toHaveText(paused ?? "");
  await page.getByRole("button", { name: "Resume", exact: true }).click();
  await expect(output).toContainText(`fixture heartbeat ${nextHeartbeat}`);
  await page.getByRole("combobox", { name: "Choose log source" }).click();
  await page
    .getByRole("option", { name: "IDE engine (attached)", exact: true })
    .click();
  await expect(output).toContainText(
    "Live logs unavailable for attached engines",
  );
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Download", exact: true }),
  ).toBeDisabled();
});

for (const width of [240, 360, 1280]) {
  test(`console tour and navigation remain accessible at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/tests/e2e/daemon-console.html?theme=light&page=chats");
    await expect(
      page.getByText("Improve daemon console", { exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Waiting", exact: true }).click();
    await expect(
      page.getByText("Improve daemon console", { exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByText("Review component tokens", { exact: true }),
    ).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
      )
      .toBe(true);
    const tour = page.getByRole("button", { name: "Quick tour", exact: true });
    await tour.click();
    await expect(page.getByRole("dialog")).toContainText("Projects · 1 of 4");
    await page.getByRole("button", { name: "Next", exact: true }).click();
    await expect(page.getByRole("dialog")).toContainText("Engines · 2 of 4");
    await page.getByRole("button", { name: "Back", exact: true }).click();
    await page.getByRole("button", { name: "Skip", exact: true }).click();
    await expect(tour).toBeFocused();
    await tour.click();
    await page.keyboard.press("Escape");
    await expect(tour).toBeFocused();
    if (width < 720) {
      await page
        .getByRole("button", { name: "Open navigation", exact: true })
        .click();
      await page.getByRole("button", { name: "Engines", exact: true }).click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
    } else {
      await page.getByRole("button", { name: "Engines", exact: true }).click();
    }
    await page
      .getByRole("button", { name: "Connect engine", exact: true })
      .click();
    await page.getByLabel("Port", { exact: true }).fill("18799");
    await page
      .getByLabel("Label (optional)", { exact: true })
      .fill("Paired browser fixture");
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    const row = page
      .locator("article")
      .filter({ hasText: "Paired browser fixture" });
    await expect(row).toBeVisible();
    await expect(row.getByRole("button")).toHaveCount(1);
    await row.getByRole("button", { name: "Disconnect", exact: true }).click();
    await expect(row).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => document.documentElement.scrollWidth <= innerWidth),
      )
      .toBe(true);
  });
}
