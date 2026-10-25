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

test("compact native protocol keeps decisions draft-only after explicit opening and preserves send retries", async ({
  page,
}) => {
  const commands: {
    client_request_id: string;
    type: string;
    response: unknown;
  }[] = [];
  await page.route("**/v1/chats/*/commands", async (route) => {
    commands.push(route.request().postDataJSON() as (typeof commands)[number]);
    await route.fulfill({
      status: commands.length === 1 ? 503 : 200,
      contentType: "application/json",
      body: commands.length === 1 ? '{"error":"fixture offline"}' : "{}",
    });
  });
  await page.goto("/tests/e2e/questions-showcase.html?native=pending");
  const row = page.getByRole("button", {
    name: /Choose the next direction.*Needs answer/,
  });
  await expect(row).toHaveAttribute("aria-expanded", "false");
  await page.getByRole("button", { name: "Before question" }).focus();
  await page.keyboard.press("Tab");
  await expect(row).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "After question" }),
  ).toBeFocused();
  await row.focus();
  await page.keyboard.press("Enter");
  await expect(row).toHaveAttribute("aria-expanded", "true");
  await expect(page.getByRole("button", { name: "More details" })).toHaveCount(
    0,
  );
  await expect(
    page.getByRole("button", { name: "Submit response" }),
  ).toBeDisabled();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Skip", exact: true }).first(),
  ).toBeFocused();
  await page
    .getByRole("button", { name: "Write my own answer" })
    .first()
    .click();
  await page
    .getByRole("textbox", { name: "Your answer to question 1" })
    .fill("Combine both");
  await page.getByRole("button", { name: "Add explanation" }).first().click();
  await page
    .getByRole("textbox", { name: "Explanation for question 1" })
    .fill("A deliberate choice");
  await page.getByRole("button", { name: "Skip", exact: true }).nth(1).click();
  await page
    .frameLocator('iframe[title="HTML Preview"]')
    .getByRole("button", { name: "Use Mint layout" })
    .click();
  await expect(
    page.getByRole("textbox", { name: "Your answer to question 3" }),
  ).toHaveValue("Mint layout");
  await page
    .getByRole("textbox", { name: /Overall comments/ })
    .fill("Keep the full history");
  expect(commands).toEqual([]);
  await page.getByRole("button", { name: "Submit response" }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await row.click();
  await expect(row).toContainText("Send failed");
  await row.click();
  await page.getByRole("button", { name: "Submit response" }).click();
  await expect(
    page.getByText("Response sent, awaiting confirmation."),
  ).toBeVisible();
  expect(commands).toHaveLength(2);
  expect(commands[0].client_request_id).toBe(commands[1].client_request_id);
  expect(commands[1]).toMatchObject({
    type: "submit_question_response",
    response: {
      answers: [
        {
          id: "direction",
          disposition: "custom",
          answer: "Combine both",
          explanation: "A deliberate choice",
        },
        { id: "permission", disposition: "skipped" },
        { id: "artifact", disposition: "custom", answer: "Mint layout" },
      ],
      overall_comments: "Keep the full history",
    },
  });
  await expect(
    page.getByRole("button", { name: "Submit response" }),
  ).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Check status / retry" }),
  ).toBeVisible();
  await row.click();
  await expect(row).toContainText("Needs answer · Awaiting confirmation");
});

for (const status of ["answered", "cancelled"] as const) {
  test(`compact native ${status} is historical and keeps complete content recoverable`, async ({
    page,
  }) => {
    await page.goto(`/tests/e2e/questions-showcase.html?native=${status}`);
    const row = page.getByRole("button", { expanded: false });
    await expect(row).toContainText(
      status === "answered" ? "Answered" : "Withdrawn",
    );
    await row.click();
    const details = page.getByRole("button", { name: "More details" });
    await expect(details).toHaveAttribute("aria-expanded", "false");
    const body = page.locator(
      `[id="${await details.getAttribute("aria-controls")}"]`,
    );
    await expect(body).toHaveAttribute("inert", "");
    await expect
      .poll(() =>
        body.evaluate((element) => element.getBoundingClientRect().height),
      )
      .toBe(0);
    await details.focus();
    await page.keyboard.press("Tab");
    await expect(
      page.getByRole("button", { name: "After question" }),
    ).toBeFocused();
    await details.click();
    await expect(page.getByText("Roomy", { exact: true })).toBeVisible();
    if (status === "answered") {
      await expect(page.getByText("Custom answer: Combine both")).toBeVisible();
      await expect(page.getByText("A deliberate choice")).toBeVisible();
      await expect(page.getByText("Skipped", { exact: true })).toBeVisible();
      await expect(page.getByText("Keep the full history")).toBeVisible();
    } else {
      await expect(page.getByText("No response submitted")).toHaveCount(3);
    }
    await expect(
      page.getByRole("button", { name: "Submit response" }),
    ).toHaveCount(0);
  });
}

test("compact native permission dismissal remains an explicit skipped response, never approval", async ({
  page,
}) => {
  const commands: {
    response: { answers: { disposition: string; answer?: unknown }[] };
  }[] = [];
  await page.route("**/v1/chats/*/commands", async (route) => {
    commands.push(route.request().postDataJSON() as (typeof commands)[number]);
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "{}",
    });
  });
  await page.goto("/tests/e2e/questions-showcase.html?native=approval");
  await page.getByRole("button", { expanded: false }).click();
  await expect(
    page.getByText(/Only an explicit Approve grants permission/),
  ).toBeVisible();
  await page.getByRole("button", { name: "Skip remaining" }).click();
  expect(commands).toEqual([]);
  await page.getByRole("button", { name: "Submit response" }).click();
  await expect(
    page.getByText("Response sent, awaiting confirmation."),
  ).toBeVisible();
  expect(commands).toHaveLength(1);
  expect(
    commands[0].response.answers.every(
      (answer) => answer.disposition === "skipped" && !("answer" in answer),
    ),
  ).toBe(true);
});
