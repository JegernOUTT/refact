import { expect, test } from "@playwright/test";

test("AtCommandChip activates by click, Enter and Space only when enabled", async ({
  page,
}) => {
  await page.goto("/tests/e2e/design-token-disabled-opacity.html");

  const disabled = page.getByRole("button", { name: "Disabled file" });
  const enabled = page.getByRole("button", { name: "Enabled file" });
  await expect(disabled).toHaveAttribute("aria-disabled", "true");
  await expect(enabled).toHaveAttribute("aria-disabled", "false");
  await expect(disabled).toHaveAttribute("tabindex", "-1");
  await expect(enabled).toHaveAttribute("tabindex", "0");

  await page.getByRole("button", { name: "Before chips" }).focus();
  await page.keyboard.press("Tab");
  await expect(enabled).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "After chips" })).toBeFocused();

  const disabledCalls = page.getByTestId("disabled-calls");
  const enabledCalls = page.getByTestId("enabled-calls");
  await expect(disabledCalls).toHaveText("0");
  await expect(enabledCalls).toHaveText("0");

  // aria-disabled on this span does not suppress browser pointer events.
  // Use real mouse input to check its handler rather than Playwright's
  // higher-level refusal to activate an aria-disabled locator.
  const disabledBox = await disabled.boundingBox();
  if (!disabledBox) throw new Error("Disabled chip has no pointer target");
  await page.mouse.click(
    disabledBox.x + disabledBox.width / 2,
    disabledBox.y + disabledBox.height / 2,
  );
  await expect(disabledCalls).toHaveText("0");
  await enabled.click();
  await expect(enabledCalls).toHaveText("1");

  for (const [index, key] of ["Enter", "Space"].entries()) {
    // Disabled chips are intentionally absent from sequential focus, but must
    // still reject activation if focused programmatically.
    await disabled.focus();
    await expect(disabled).toBeFocused();
    await disabled.press(key);
    await expect(disabledCalls).toHaveText("0");
    await expect(enabledCalls).toHaveText(String(index + 1));

    await enabled.focus();
    await expect(enabled).toBeFocused();
    await enabled.press(key);
    await expect(enabledCalls).toHaveText(String(index + 2));
    await expect(disabledCalls).toHaveText("0");
  }
});
