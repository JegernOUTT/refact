import { expect, test } from "@playwright/test";

test("mixed native and sandboxed answers stay draft-only until explicit submission", async ({
  page,
}) => {
  const external: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("blocked.invalid")) external.push(request.url());
  });
  await page.goto("/tests/e2e/questions-showcase.html");
  await page
    .getByRole("button", { name: "Write my own answer" })
    .first()
    .click();
  await page
    .getByRole("textbox", { name: "Your answer to question 1" })
    .fill("A mix of both");
  await page.getByRole("button", { name: "Add explanation" }).first().click();
  await page
    .getByRole("textbox", { name: "Explanation for question 1" })
    .fill("Desktop first");
  await page.getByRole("button", { name: "None of these" }).click();
  await page
    .frameLocator('iframe[title="HTML Preview"]')
    .getByRole("button", { name: "Use Mint layout" })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Your answer to question 3" }),
  ).toHaveValue("Mint accent, 12px spacing");
  await page
    .getByRole("textbox", { name: "Your answer to question 3" })
    .fill("Private native answer");
  const preview = page.frames().find((frame) => frame !== page.mainFrame());
  expect(preview).toBeTruthy();
  expect(
    await preview?.evaluate(() =>
      (
        window as unknown as { refactQuestion: { getAnswer: () => string } }
      ).refactQuestion.getAnswer(),
    ),
  ).toBe("Mint accent, 12px spacing");
  await expect(page.getByTestId("question-response")).toHaveCount(0);
  await page.getByRole("button", { name: "Skip", exact: true }).first().click();
  await page.getByRole("button", { name: "Undo skip" }).click();
  await expect(
    page.getByRole("textbox", { name: "Your answer to question 1" }),
  ).toHaveValue("A mix of both");
  await page
    .getByRole("textbox", { name: /Overall comments/ })
    .fill("Keep it small");
  await page.getByRole("button", { name: "Submit response" }).click();
  const payload = JSON.parse(
    await page.getByTestId("question-response").innerText(),
  ) as { answers: unknown[]; overall_comments: string };
  expect(payload).toMatchObject({
    answers: [
      {
        id: "direction",
        disposition: "custom",
        answer: "A mix of both",
        explanation: "Desktop first",
      },
      { id: "features", disposition: "answered", answer: [] },
      { id: "preview", disposition: "custom", answer: "Private native answer" },
    ],
    overall_comments: "Keep it small",
  });
  await page
    .frameLocator('iframe[title="HTML Preview"]')
    .getByRole("button", { name: "Use Violet layout" })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Your answer to question 3" }),
  ).toHaveValue("Private native answer");
  expect(external).toEqual([]);
});

for (const width of [240, 360, 768, 1280]) {
  test(`all-skip and narrow layout at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/tests/e2e/questions-showcase.html");
    await page.getByRole("button", { name: "Skip remaining" }).click();
    await expect(
      page.getByText("0 answered · 3 skipped · 0 remaining"),
    ).toBeVisible();
    await page.getByRole("button", { name: "Submit response" }).click();
    const payload = JSON.parse(
      await page.getByTestId("question-response").innerText(),
    ) as { answers: { disposition: string; answer?: unknown }[] };
    expect(
      payload.answers.every(
        (a) => a.disposition === "skipped" && !("answer" in a),
      ),
    ).toBe(true);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
    ).toBe(false);
  });
}
