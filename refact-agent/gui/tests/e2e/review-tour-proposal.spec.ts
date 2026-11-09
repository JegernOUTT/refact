import { expect, test } from "@playwright/test";

// This checks the production chat renderer against authored fixture data, not an engine.
for (const [width, theme] of [
  [390, "light"],
  [1280, "dark"],
] as const) {
  test(`synthetic acceptance survives reload and authored fulfilment (${width}, ${theme})`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(
      `/tests/e2e/review-tour-proposal.html?key=${width}&theme=${theme}`,
    );
    await expect(page.getByText("Canonical synthetic proposal")).toBeVisible();
    await expect(page.getByText("Proposed", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Accept Tour revision" }).click();
    await expect(
      page.getByText("Accepted pending", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText(/Acceptance does not start work/),
    ).toBeVisible();
    let state = await page.evaluate(() => window.__proposalFixture.snapshot());
    expect(state.review.tour[0].current.version).toBe(1);
    expect(state.review.tour[0].history).toHaveLength(0);
    expect(state.posts).toHaveLength(1);
    expect(state.posts[0]).toEqual({
      path: "/v1/reviews/r1/tour-revision-proposals/p1/accept",
      body: {
        owner_chat_id: "chat-1",
        expected_revision: 3,
        request_key: expect.any(String),
        target: { candidate_id: "c1", step_id: "s1", version: 1 },
      },
    });
    await page.reload();
    await expect(
      page.getByText("Accepted pending", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Accept Tour revision" }),
    ).toHaveCount(0);
    expect(
      (await page.evaluate(() => window.__proposalFixture.snapshot())).posts,
    ).toHaveLength(1);
    await page.evaluate(() => window.__proposalFixture.fulfil());
    await expect(page.getByText("Fulfilled", { exact: true })).toBeVisible();
    await expect(page.getByText(/Stale|Accepted pending/)).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Accept Tour revision" }),
    ).toHaveCount(0);
    state = await page.evaluate(() => window.__proposalFixture.snapshot());
    expect(state.review.tour[0].current.version).toBe(2);
    expect(state.review.tour[0].history[0].version).toBe(1);
    expect(state.review.tour_revision_proposals?.[0].fulfilment?.version).toBe(
      2,
    );
    expect(state.posts).toHaveLength(1);
  });
}
