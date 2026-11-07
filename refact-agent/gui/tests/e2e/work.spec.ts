import { test, expect } from "@playwright/test";
import { workFixture } from "../../src/services/refact/workFixtures";

for (const width of [360, 1280]) {
  test(`Work discovery states and paging remain usable at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.route("**/v1/coordination-settings?*", (route) =>
      route.fulfill({ json: { effective: {} } }),
    );
    await page.route("**/v1/work*", (route) => {
      const second = new URL(route.request().url()).searchParams.has("cursor");
      return route.fulfill({
        json: {
          ...workFixture,
          total: 2,
          truncated: !second,
          next_cursor: second ? undefined : "w1.next",
          rows: [
            {
              ...workFixture.rows[0],
              title: second ? "Parked child" : "Queued board",
              state: second ? "parked" : "queued",
              attention: second,
            },
          ],
        },
      });
    });
    await page.goto("/tests/e2e/route-showcase.html?route=work");
    await expect(page.getByText("Queued board")).toBeVisible();
    await expect(page.getByText("Queued", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Next page" }).click();
    await expect(page.getByText("Parked child")).toBeVisible();
    await expect(page.getByText("Needs attention")).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Next page" }),
    ).toBeDisabled();
    await page.getByText(/File activity/).click();
    await expect(
      page.getByText(/Declared intent is not a file lock/),
    ).toBeVisible();
    await page.getByRole("button", { name: "First page" }).click();
    await expect(page.getByText("Queued board")).toBeVisible();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
  });
}

for (const manualRetry of [false, true]) {
  test(`Work stream gaps hide old rows and reconnect with a fresh snapshot (${
    manualRetry ? "manual" : "automatic"
  })`, async ({ page }) => {
    await page.addInitScript(() => {
      const original = window.fetch.bind(window);
      const state = {
        controllers: [] as ReadableStreamDefaultController<Uint8Array>[],
      };
      Object.assign(window, { workStreams: state });
      window.fetch = (input, init) => {
        const url =
          typeof input === "string"
            ? input
            : input instanceof URL
              ? input.toString()
              : input.url;
        if (url.includes("/v1/work/subscribe"))
          return Promise.resolve(
            new Response(
              new ReadableStream<Uint8Array>({
                start(controller) {
                  state.controllers.push(controller);
                },
              }),
              { headers: { "Content-Type": "text/event-stream" } },
            ),
          );
        return original(input, init);
      };
    });
    await page.route("**/v1/coordination-settings?*", (route) =>
      route.fulfill({
        json: {
          effective: {
            work_stream_timeout_ms: 60_000,
            work_reconnect_max_ms: manualRetry ? 60_000 : 100,
          },
        },
      }),
    );
    let releaseRefresh!: () => void;
    const refreshReady = new Promise<void>((resolve) => {
      releaseRefresh = resolve;
    });
    let requests = 0;
    await page.route("**/v1/work", async (route) => {
      requests += 1;
      // Refresh also refetches REST. Hold its older snapshot so this assertion
      // exercises stream recovery, then verify it cannot replace the fresh SSE.
      if (requests > 1) await refreshReady;
      await route.fulfill({ json: workFixture });
    });
    await page.goto("/tests/e2e/route-showcase.html?route=work&theme=light");
    await expect(page.getByText("Board work")).toBeVisible();
    await page.waitForFunction(
      () =>
        (window as unknown as { workStreams: { controllers: unknown[] } })
          .workStreams.controllers.length === 1,
    );
    await page.evaluate((fixture) => {
      const controller = (
        window as unknown as {
          workStreams: {
            controllers: ReadableStreamDefaultController<Uint8Array>[];
          };
        }
      ).workStreams.controllers[0];
      for (const seq of [1, 3])
        controller.enqueue(
          new TextEncoder().encode(
            `event: snapshot\ndata: ${JSON.stringify({ ...fixture, seq })}\n\n`,
          ),
        );
    }, workFixture);
    await expect(page.getByRole("alert")).toContainText("Work unavailable");
    await expect(page.getByText("Board work")).toHaveCount(0);
    if (manualRetry)
      await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await page.waitForFunction(
      () =>
        (window as unknown as { workStreams: { controllers: unknown[] } })
          .workStreams.controllers.length === 2,
    );
    if (manualRetry) await expect.poll(() => requests).toBe(2);
    await expect(page.getByText("Board work")).toHaveCount(0);
    await page.evaluate((fixture) => {
      const controller = (
        window as unknown as {
          workStreams: {
            controllers: ReadableStreamDefaultController<Uint8Array>[];
          };
        }
      ).workStreams.controllers[1];
      controller.enqueue(
        new TextEncoder().encode(
          `event: snapshot\ndata: ${JSON.stringify({
            ...fixture,
            seq: 4,
            rows: [{ ...fixture.rows[0], title: "Fresh work" }],
          })}\n\n`,
        ),
      );
    }, workFixture);
    await expect(page.getByText("Fresh work")).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
    if (manualRetry) {
      const refreshed = page.waitForResponse("**/v1/work");
      releaseRefresh();
      await (await refreshed).finished();
      await expect(
        page.getByRole("button", { name: "Refresh project work", exact: true }),
      ).toBeEnabled();
      await expect(page.getByText("Fresh work")).toBeVisible();
      await expect(page.getByText("Board work")).toHaveCount(0);
    }
  });
}
