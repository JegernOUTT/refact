import { test, type Page } from "@playwright/test";

export async function openReviewFixture(page: Page, url: string) {
  await page.goto(url, { waitUntil: "domcontentloaded" });
  const testTimeout = test.info().timeout;
  const timeout = testTimeout > 0 ? Math.min(testTimeout, 30_000) : 30_000;
  await page.waitForFunction(
    () => {
      const fixture = (
        window as Window & {
          __REFACT_REVIEW_FIXTURE__?: {
            status: string;
            message?: string;
            stack?: string | null;
          };
        }
      ).__REFACT_REVIEW_FIXTURE__;
      if (fixture?.status === "FAIL") {
        throw new Error(
          `Review fixture FAIL: ${fixture.message ?? "unknown error"}\n${
            fixture.stack ?? ""
          }`,
        );
      }
      return (
        location.pathname === "/tests/e2e/review-canvas.html" &&
        fixture?.status === "ready"
      );
    },
    undefined,
    { timeout },
  );
}
