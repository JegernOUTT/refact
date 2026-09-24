import { expect, test } from "@playwright/test";

test("disabled AtCommandChip opacity follows the shared semantic token", async ({
  page,
}) => {
  await page.goto("/tests/e2e/design-token-disabled-opacity.html");

  const disabled = page.getByRole("button", { name: "Disabled file" });
  const enabled = page.getByRole("button", { name: "Enabled file" });
  await expect(disabled).toHaveAttribute("aria-disabled", "true");
  await expect(enabled).toHaveAttribute("aria-disabled", "false");
  await expect(enabled).toHaveCSS("opacity", "1");
  const disabledOpacity = Number.parseFloat(
    await disabled.evaluate((element) => getComputedStyle(element).opacity),
  );
  expect(disabledOpacity).toBeLessThan(1);
});
