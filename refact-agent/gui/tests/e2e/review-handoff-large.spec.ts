import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";

test("denied clipboard downloads the entire large snapshot without mounting it", async ({
  page,
}) => {
  await page.route("**/handoff-fixture", async (route) => {
    await route.fulfill({
      contentType: "text/html; charset=utf-8",
      body: `<!doctype html><div id="root"></div><script type="module">import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => (type) => type; window.__vite_plugin_react_preamble_installed__ = true;</script><script type="module">
      import React from '/node_modules/.vite/deps/react.js';
      import ReactDOM from '/node_modules/.vite/deps/react-dom_client.js';
      const { createRoot } = ReactDOM;
      import { ReviewHandoff } from '/src/features/Review/ReviewHandoff.tsx';
      const markdown = 'é🦊\\n' + 'x'.repeat(20 * 1024 * 1024);
      window.handoffStats = { writes: 0, created: [], revoked: [] };
      const createURL = URL.createObjectURL.bind(URL);
      const revokeURL = URL.revokeObjectURL.bind(URL);
      URL.createObjectURL = (blob) => { const url = createURL(blob); window.handoffStats.created.push(url); return url; };
      URL.revokeObjectURL = (url) => { window.handoffStats.revoked.push(url); revokeURL(url); };
      Object.defineProperty(navigator, 'clipboard', { value: { writeText: () => { window.handoffStats.writes++; return Promise.reject(Error('denied')); } } });
      document.execCommand = () => { throw Error('unexpected execCommand'); };
      HTMLTextAreaElement.prototype.select = () => { throw Error('unexpected selection'); };
      createRoot(document.getElementById('root')).render(React.createElement(ReviewHandoff, {reviewId:'r1',revision:3,load: async () => ({review_id:'r1',revision:3,stack_revision:8,scope:'whole_stack',filename:'review.md',markdown})}));
    </script>`,
    });
  });
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await page.goto("/handoff-fixture");
  await page.getByRole("button", { name: "Copy review as Markdown" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator("textarea")).toHaveCount(0);
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    dialog.getByRole("button", { name: "Download .md" }).click(),
  ]);
  const path = await download.path();
  expect(path).not.toBeNull();
  const bytes = await readFile(path!);
  const expected = Buffer.from("é🦊\n" + "x".repeat(20 * 1024 * 1024));
  expect(bytes.length).toBe(expected.length);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  expect(sha256).toBe(createHash("sha256").update(expected).digest("hex"));
  expect(download.suggestedFilename()).toBe("review.md");
  await expect(page.locator('a[href^="blob:"]')).toHaveCount(0);
  await expect(
    page.getByText("Review Markdown copied", { exact: true }),
  ).toHaveCount(0);
  await dialog
    .getByRole("button", { name: "Close", exact: true })
    .click({ timeout: 2000 });
  await expect(dialog).not.toBeVisible({ timeout: 2000 });
  await page
    .getByRole("button", { name: "Copy review as Markdown" })
    .click({ timeout: 2000 });
  await expect(dialog).toBeVisible({ timeout: 2000 });
  await expect(page.locator("textarea")).toHaveCount(0);
  await expect(
    dialog.getByRole("button", { name: "Show full Markdown for manual copy" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible({ timeout: 2000 });
  await expect
    .poll(() =>
      page.evaluate(() => {
        const stats = (
          window as unknown as {
            handoffStats: {
              created: string[];
              revoked: string[];
              writes: number;
            };
          }
        ).handoffStats;
        return {
          writes: stats.writes,
          cleaned:
            stats.created.length === 1 &&
            stats.created.every((url) => stats.revoked.includes(url)),
        };
      }),
    )
    .toEqual({ writes: 2, cleaned: true });
  expect(pageErrors).toEqual([]);
  console.log(
    JSON.stringify({
      bytes: bytes.length,
      sha256,
      closeAndReopen: "passed",
      blobCleanup: "passed",
    }),
  );
});
