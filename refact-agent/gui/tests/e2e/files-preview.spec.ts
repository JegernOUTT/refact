import { expect, test } from "@playwright/test";

test("opens an external image read-only with working preview controls", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/tests/e2e/route-showcase.html?route=workspace-chrome");
  await page.getByLabel("Absolute file path", { exact: true }).waitFor();
  const imageData = await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 2000;
    canvas.height = 1000;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas context unavailable");
    context.fillRect(0, 0, canvas.width, canvas.height);
    return canvas.toDataURL("image/png").split(",")[1];
  });
  const requests: { method: string; body: unknown }[] = [];
  await page.route("**/v1/files/open", async (route) => {
    requests.push({
      method: route.request().method(),
      body: route.request().postDataJSON() as unknown,
    });
    await route.fulfill({
      json: {
        path: "/tmp/external-preview.png",
        content: "",
        language: null,
        size: 8000,
        truncated: false,
        line_start: null,
        line_end: null,
        mtime_ms: 1,
        binary: true,
        can_edit: false,
        image_mime: "image/png",
        image_data: imageData,
      },
    });
  });
  await page
    .getByLabel("Absolute file path", { exact: true })
    .fill("/tmp/external-preview.png");
  await page.getByRole("button", { name: "Open", exact: true }).click();
  const image = page.getByRole("img", { name: "external-preview.png" });
  await expect(image).toBeVisible();
  await expect
    .poll(() =>
      image.evaluate((element) => (element as HTMLImageElement).naturalWidth),
    )
    .toBe(2000);
  expect(requests).toEqual([
    { method: "POST", body: { path: "/tmp/external-preview.png" } },
  ]);
  await expect(
    page.getByRole("button", { name: "Edit this file" }),
  ).toBeDisabled();
  await expect(page.getByText("Read-only", { exact: true })).toBeVisible();
  const zoom = page.getByLabel("Zoom level", { exact: true });
  await page.getByRole("button", { name: "Actual size", exact: true }).click();
  await expect(zoom).toHaveText("100%");
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  await expect(zoom).toHaveText("125%");
  await page.getByRole("button", { name: "Rotate clockwise" }).click();
  await expect(image).toHaveAttribute("data-rotation", "90");
  const resize = page.getByRole("separator", { name: "Resize image preview" });
  await resize.focus();
  await resize.press("ArrowDown");
  await expect(resize).toHaveAttribute("aria-valuenow", "400");
  await page.getByRole("button", { name: "Fit to view", exact: true }).click();
  await expect
    .poll(() => image.boundingBox().then((box) => box?.height ?? 0))
    .toBeLessThanOrEqual(401);
  await page.setViewportSize({ width: 360, height: 900 });
  await expect(image).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth + 1,
      ),
    )
    .toBe(true);
});
