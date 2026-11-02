import { expect, test } from "@playwright/test";

test.use({ reducedMotion: "reduce" });
for (const theme of ["dark", "light"]) {
  for (const width of [240, 360, 768, 1280]) {
    test(`connected Buddy stays usable at ${width}px ${theme} without navigating the source`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(`/tests/e2e/buddy-companion.html?theme=${theme}`);
      const mainDraft = page.getByRole("textbox", {
        name: "Main source draft",
      });
      await mainDraft.focus();
      await expect(
        page.getByLabel("Buddy attention", { exact: true }),
      ).toBeVisible();
      await expect(mainDraft).toBeFocused();
      await page
        .getByRole("button", { name: "Ask Buddy, 1 attention requests" })
        .click();
      await expect(page.getByRole("dialog", { name: "Buddy" })).toBeVisible();
      await expect(page.getByTestId("chat-form-textarea")).toBeVisible();
      await page
        .getByTestId("chat-form-textarea")
        .fill("Explain this source's status");
      await page.getByRole("tab", { name: "Needs attention (1)" }).click();
      await expect(
        page.getByText("Verify package changes — Tool permission"),
      ).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Approve once" }),
      ).toBeVisible();
      await page.getByRole("button", { name: "Approve once" }).click();
      await expect(page.getByText(/Decision: Queued/)).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Approve once" }),
      ).toBeDisabled();
      await page.getByRole("tab", { name: "Ask Buddy", exact: true }).click();
      await expect(page.getByTestId("chat-form-textarea")).toHaveValue(
        "Explain this source's status",
      );
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog", { name: "Buddy" })).toBeHidden();
      await expect(
        page.getByRole("button", { name: "Ask Buddy, 1 attention requests" }),
      ).toBeFocused();
      await expect(mainDraft).toHaveValue(
        "Keep my investigation notes unchanged",
      );
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
      ).toBe(true);
      await page.screenshot({
        path: `test-results/buddy-companion-${width}-${theme}.png`,
      });
    });
  }
}

test("compact narrow permission can be reached by keyboard without opening a modal or changing the source draft", async ({
  page,
}) => {
  await page.setViewportSize({ width: 240, height: 900 });
  await page.goto("/tests/e2e/buddy-companion.html");
  const approve = page.getByRole("button", { name: "Approve once" });
  await expect(approve).toBeAttached();
  await page.getByRole("button", { name: "Review selected request" }).focus();
  for (let index = 0; index < 8; index++) {
    if (await approve.evaluate((element) => element === document.activeElement))
      break;
    await page.keyboard.press("Tab");
  }
  await expect(approve).toBeFocused();
  await page.keyboard.press("Space");
  await expect(page.getByText(/Decision: Queued/)).toBeVisible();
  await expect(page.getByRole("dialog")).toBeHidden();
  await expect(
    page.getByRole("textbox", { name: "Main source draft" }),
  ).toHaveValue("Keep my investigation notes unchanged");
});
test("desktop window supports keyboard move resize reset and Escape restores focus", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("/tests/e2e/buddy-companion.html");
  await expect(
    page.getByLabel("Buddy attention", { exact: true }),
  ).toBeVisible();
  const launcher = page.getByRole("button", {
    name: "Ask Buddy, 1 attention requests",
  });
  await launcher.click();
  const dialog = page.getByRole("dialog", { name: "Buddy", exact: true });
  await page.getByText("Window position and size", { exact: true }).click();
  const initial = await dialog.boundingBox();
  expect(initial).not.toBeNull();
  await page.getByRole("button", { name: "Move left", exact: true }).focus();
  await page.keyboard.press("Space");
  await expect
    .poll(async () => (await dialog.boundingBox())?.x)
    .toBeLessThan(initial!.x);
  await page.getByRole("button", { name: "Smaller", exact: true }).focus();
  await page.keyboard.press("Space");
  await expect
    .poll(async () => (await dialog.boundingBox())?.width)
    .toBeLessThan(initial!.width);
  await page.getByRole("button", { name: "Reset window", exact: true }).focus();
  await page.keyboard.press("Space");
  await expect
    .poll(async () => (await dialog.boundingBox())?.width)
    .toBe(initial!.width);
  await page.keyboard.press("Escape");
  await expect(launcher).toBeFocused();
});

for (const width of [360, 1280]) {
  test(`question drafts and main draft stay editable with nonmodal Buddy at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/tests/e2e/buddy-companion.html?question=1");
    const main = page.getByRole("textbox", { name: "Main source draft" });
    const compact = page.getByLabel("Buddy attention", { exact: true });
    const answer = compact.getByRole("textbox", {
      name: "Your answer to question 1",
    });
    await expect(answer).toBeVisible();
    await answer.pressSequentially("A compact draft");
    await expect(answer).toHaveValue("A compact draft");
    await page
      .getByRole("button", { name: "Ask Buddy, 1 attention requests" })
      .click();
    const dialog = page.getByRole("dialog", { name: "Buddy" });
    const composer = dialog.getByTestId("chat-form-textarea");
    await composer.fill("Initialize Buddy");
    await dialog.getByRole("button", { name: "Send message" }).click();
    await expect(dialog.getByText("Buddy shared response")).toBeVisible();
    const nativeAnswer = dialog.getByRole("textbox", {
      name: "Your answer to question 1",
    });
    await nativeAnswer.pressSequentially("Native draft");
    await expect(nativeAnswer).toHaveValue("Native draft");
    await composer.fill("A composer draft");
    const headerBox = await dialog
      .getByRole("button", { name: "Minimize Buddy" })
      .boundingBox();
    const composerBox = await composer.boundingBox();
    expect(headerBox).not.toBeNull();
    expect(composerBox).not.toBeNull();
    expect(headerBox!.y).toBeLessThan(composerBox!.y);
    await dialog.getByRole("button", { name: "Minimize Buddy" }).click();
    await expect(dialog).toBeHidden();
    await page
      .getByRole("button", { name: "Ask Buddy, 1 attention requests" })
      .click();
    await expect(composer).toHaveValue("A composer draft");
    await expect(nativeAnswer).toHaveValue("Native draft");
    await dialog.getByRole("tab", { name: "Needs attention (1)" }).click();
    await expect(
      dialog
        .getByLabel("Selected attention request")
        .getByRole("textbox", { name: "Your answer to question 1" }),
    ).toHaveValue("A compact draft");
    await main.fill("Main draft remains editable");
    await expect(main).toHaveValue("Main draft remains editable");
    expect(
      await page.evaluate(
        () => (window as unknown as { fixturePosts: string[] }).fixturePosts,
      ),
    ).toEqual(["/v1/buddy/companion/source-a"]);
  });
}
