import { expect, test } from "@playwright/test";

for (const width of [360, 1280]) {
  test(`expanded App completions and multimodal events render at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto("/tests/e2e/app-tool-results.html");

    const apps = page.getByTestId("app-tool");
    await expect(apps).toHaveCount(2);
    const resource = apps.nth(0);
    const capture = apps.nth(1);
    await expect(resource.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    await expect(resource.getByText("Resource", { exact: true })).toBeVisible();
    await expect(
      resource.getByText("ui/index.html", { exact: true }),
    ).toBeVisible();
    const contentWidth = await resource
      .locator("dd")
      .filter({ hasText: "<!doctype html>" })
      .evaluate((element) => element.getBoundingClientRect().width);
    const resourceWidth = await resource.evaluate(
      (element) => element.getBoundingClientRect().width,
    );
    expect(contentWidth).toBeGreaterThan(resourceWidth / 2);
    await expect(capture.getByTestId("transcript-meta")).toContainText(
      "PNG captured",
    );
    const image = capture.locator("img");
    await expect(image).toBeVisible();
    await expect(image).toHaveJSProperty("naturalWidth", 1);

    const notice = page.getByTestId("event-row");
    await expect(notice).toHaveCount(1);
    await notice.getByTestId("transcript-toggle").click();
    await expect(notice.locator("img")).toBeVisible();
    await resource.getByRole("button", { name: "Raw JSON" }).click();
    await expect(page.locator("body")).not.toContainText("HIDDEN_TOKEN");
    await expect(page.locator("body")).not.toContainText("[object Object]");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(errors).toEqual([]);
  });
}
