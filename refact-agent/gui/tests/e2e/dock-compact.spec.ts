import { expect, test, type Page } from "@playwright/test";

const SESSIONS = [
  {
    id: "first-123456",
    cmd: "Record the pinned benchmark protocol",
    tty: false,
  },
  { id: "second-12345", cmd: "/bin/zsh", tty: true },
];

async function installExecMock(page: Page) {
  await page.evaluate((sessions) => {
    const nativeFetch = window.fetch.bind(window);
    const json = (body: unknown) =>
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    window.fetch = (input, init) => {
      const url = new URL(
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url,
        window.location.href,
      );
      if (url.pathname === "/v1/exec/spawn") {
        return Promise.resolve(
          json({
            process_id: "spawned-1234",
            status: "running",
            command_preview: "/bin/bash",
          }),
        );
      }
      if (url.pathname.startsWith("/v1/exec/")) {
        return Promise.resolve(
          json({ chunks: [], next_seq: 0, status: "running", processes: [] }),
        );
      }
      return nativeFetch(input, init);
    };
    void sessions;
  }, SESSIONS);
}

for (const [width, scheme] of [
  [1200, "dark"],
  [1200, "light"],
] as const) {
  test.describe(`terminal tabs toggle the panel ${width}px ${scheme}`, () => {
    test.use({ viewport: { width, height: 800 }, colorScheme: scheme });

    test("+ and tabs toggle, no chevron", async ({ page }) => {
      await page.goto("/tests/e2e/route-showcase.html?route=workspace-chrome");
      const panel = page.getByRole("region", {
        name: /Terminal workbench/,
      });
      await expect(panel).toBeVisible();
      await installExecMock(page);

      const body = page.getByTestId("terminal-workbench-body");
      const title = panel.getByRole("button", {
        name: "Terminal",
        exact: true,
      });
      if ((await title.getAttribute("aria-expanded")) === "true") {
        await title.click();
      }
      await expect(body).toBeHidden();
      await expect(
        panel.getByRole("button", { name: /(expand|collapse) terminal/i }),
      ).toHaveCount(0);

      await panel.getByRole("button", { name: "New terminal" }).click();
      const tab = panel.getByRole("tab").first();
      await expect(tab).toBeVisible();
      await expect(tab).toHaveAttribute("aria-expanded", "true");
      await expect(body).toBeVisible();

      await tab.click();
      await expect(body).toBeHidden();
      await expect(tab).toHaveAttribute("aria-expanded", "false");

      await tab.click();
      await expect(body).toBeVisible();

      await page.screenshot({
        path: `/tmp/rf-shots/terminal-${width}-${scheme}.png`,
      });
    });
  });
}

async function measureQueue(page: Page) {
  return page.evaluate(() => {
    const items = Array.from(
      document.querySelectorAll('[data-testid="queued-item"]'),
    );
    const item = items[0];
    const section = item?.closest("section");
    const head = section?.querySelector('[data-testid="transcript-row"]');
    const list = document.querySelector('[data-testid="queue-list"]');
    const panel = list?.parentElement?.closest("[class*='panel']");
    if (!item || !section || !head || !list || !panel)
      throw new Error("missing queue elements");
    return {
      panelWidth: panel.getBoundingClientRect().width,
      section: section.getBoundingClientRect().height,
      head: head.getBoundingClientRect().height,
      items: items.map((node) => node.getBoundingClientRect().height),
      gap:
        item.getBoundingClientRect().top - head.getBoundingClientRect().bottom,
    };
  });
}

for (const width of [1200, 480]) {
  test(`queue panel with one item is compact at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/tests/e2e/route-showcase.html?route=queue");
    const items = page.getByTestId("queued-item");
    await expect(items.first()).toBeVisible();
    while ((await items.count()) > 1) {
      const before = await items.count();
      await items
        .last()
        .getByRole("button", { name: /^Cancel/ })
        .first()
        .click();
      await expect(items).toHaveCount(before - 1);
    }
    const chip = items.first().getByRole("combobox");
    await expect(chip).toBeVisible();
    const chipBox = await chip.boundingBox();
    expect(chipBox?.height ?? 0).toBeGreaterThan(0);
    expect(chipBox?.height ?? 0).toBeLessThanOrEqual(28);

    const metrics = await measureQueue(page);
    expect(metrics.head).toBe(32);
    expect(metrics.items).toEqual([32]);
    expect(metrics.gap).toBeLessThanOrEqual(2);
    expect(metrics.section).toBeGreaterThan(0);
    expect(metrics.section).toBeLessThanOrEqual(68);
  });
}

test("choosing a queue delivery mode keeps an empty composer expanded", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.goto("/tests/e2e/route-showcase.html?route=queue");
  const items = page.getByTestId("queued-item");
  await expect(items.first()).toBeVisible();
  const textarea = page.getByTestId("chat-form-textarea");
  const form = page.locator("form").filter({ has: textarea });
  await textarea.click();
  await expect(form).toHaveClass(/chatFormExpanded/);

  const chip = items.first().locator('[role="combobox"]');
  const before = await chip.getAttribute("aria-label");
  await chip.click();
  const listbox = page.getByRole("listbox");
  await expect(listbox).toBeVisible();
  await expect(form).toHaveClass(/chatFormExpanded/);
  await listbox
    .locator('[role="option"][data-state="unchecked"]')
    .first()
    .click();
  await expect(listbox).toBeHidden();
  await expect(chip).not.toHaveAttribute("aria-label", before ?? "");
  await expect(form).toHaveClass(/chatFormExpanded/);
});
