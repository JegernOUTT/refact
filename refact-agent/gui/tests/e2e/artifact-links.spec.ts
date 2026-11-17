import { expect, test, type Page } from "@playwright/test";
import type { setUpStore } from "../../src/app/store";

const CHAT_ID = "showcase-chat";
const ARTIFACTS_DIR = `/home/user/.refact/chats/${CHAT_ID}/artifacts`;
const PNG_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

const HTML = `<!doctype html><html><body style="margin:0;font-family:sans-serif">
<header style="background:#2f6fde;color:white;padding:24px 32px"><h1 style="margin:0">Quarterly revenue</h1></header>
<main style="padding:24px 32px"><p>Revenue grew 18% quarter over quarter.</p>
<div style="display:flex;gap:12px;align-items:flex-end;height:160px">
<div style="background:#2f6fde;width:60px;height:60px"></div>
<div style="background:#2f6fde;width:60px;height:100px"></div>
<div style="background:#2f6fde;width:60px;height:140px"></div>
</div></main></body></html>`;
const CSV =
  "region,quarter,revenue,growth\nNorth,Q3,1204000,0.12\nSouth,Q3,983000,0.08\nEast,Q3,1530000,0.21\nWest,Q3,770500,0.04\n";
const MARKDOWN =
  "# Analysis notes\n\nThe **east** region outperformed every other region.\n\n- Seasonality removed\n- Currency normalized\n";

type FileEntry = {
  content: string;
  bytes: number;
  binary?: boolean;
  image?: string;
};

const FILES: Record<string, FileEntry> = {
  "reports/quarterly-revenue-dashboard.html": { content: HTML, bytes: 18_432 },
  "charts/revenue-by-region.png": {
    content: "",
    bytes: 68,
    binary: true,
    image: PNG_BASE64,
  },
  "data/revenue_by_region_q3_2026_final_cleaned_export.csv": {
    content: CSV,
    bytes: 1_572_864,
  },
  "notes/analysis.md": { content: MARKDOWN, bytes: 512 },
  "inline/summary.txt": { content: "Summary of the run\n", bytes: 19 },
};

const link = (relPath: string, title: string) =>
  `[${title}](refact://artifact/${CHAT_ID}/${relPath})`;

const MESSAGE = [
  "Here is everything the analysis produced:",
  "",
  link("reports/quarterly-revenue-dashboard.html", "Revenue dashboard"),
  "",
  link("charts/revenue-by-region.png", "Revenue by region chart"),
  "",
  link(
    "data/revenue_by_region_q3_2026_final_cleaned_export.csv",
    "revenue_by_region_q3_2026_final_cleaned_export.csv",
  ),
  "",
  link("notes/analysis.md", "Analysis notes"),
  "",
  link("missing/deleted-report.pdf", "Deleted report"),
  "",
  `The short ${link(
    "inline/summary.txt",
    "summary.txt",
  )} file sums it up in one paragraph.`,
].join("\n");

async function mockArtifacts(page: Page, reads: string[]) {
  await page.route("**/v1/artifacts/resolve?**", async (route) => {
    const url = new URL(route.request().url());
    const relPath = url.searchParams.get("path") ?? "";
    const file = FILES[relPath];
    await route.fulfill({
      json: {
        chat_id: url.searchParams.get("chat_id"),
        rel_path: relPath,
        path: `${ARTIFACTS_DIR}/${relPath}`,
        artifacts_dir: ARTIFACTS_DIR,
        exists: file !== undefined,
        bytes: file ? file.bytes : null,
        modified_at_ms: file ? 1_790_000_000_000 : null,
      },
    });
  });
  const fulfillRead = async (
    route: Parameters<Parameters<Page["route"]>[1]>[0],
    path: string,
  ) => {
    reads.push(path);
    const file = path.startsWith(`${ARTIFACTS_DIR}/`)
      ? FILES[path.slice(ARTIFACTS_DIR.length + 1)]
      : undefined;
    if (!file) {
      await route.fulfill({ status: 404, json: { detail: "File not found" } });
      return;
    }
    await route.fulfill({
      json: {
        path,
        content: file.content,
        language: null,
        size: file.bytes,
        truncated: false,
        line_start: null,
        line_end: null,
        mtime_ms: 1,
        binary: file.binary ?? false,
        can_edit: false,
        ...(file.image
          ? { image_mime: "image/png", image_data: file.image }
          : {}),
      },
    });
  };
  await page.route("**/v1/files/read?**", async (route) => {
    const url = new URL(route.request().url());
    await fulfillRead(route, url.searchParams.get("path") ?? "");
  });
  await page.route("**/v1/files/open", async (route) => {
    const body = route.request().postDataJSON() as { path?: string };
    await fulfillRead(route, body.path ?? "");
  });
}

async function renderArtifactMessage(page: Page) {
  await page.goto(
    "/tests/e2e/route-showcase.html?route=chat-dnd&toolbar_alignment&unsplit&active=0",
  );
  await page.evaluate(
    ({ chatId, content }) => {
      const store = (
        window as unknown as {
          __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
        }
      ).__toolbarAlignmentStore;
      const thread = store.getState().chat.threads[chatId]!.thread;
      store.dispatch({
        type: "chatThread/applyChatEvent",
        payload: {
          type: "snapshot",
          chat_id: chatId,
          seq: "1",
          thread,
          runtime: {
            state: "idle",
            paused: false,
            error: null,
            queue_size: 0,
            pause_reasons: [],
            queued_items: [],
          },
          messages: [
            {
              role: "user",
              message_id: "artifact-user",
              content: "Analyse the revenue data",
            },
            {
              role: "assistant",
              message_id: "artifact-assistant",
              content,
            },
          ],
          background_agents: [],
        },
      });
    },
    { chatId: CHAT_ID, content: MESSAGE },
  );
}

test("renders artifact cards and chips and opens the resolved absolute path", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  const reads: string[] = [];
  await mockArtifacts(page, reads);
  await renderArtifactMessage(page);

  const cards = page.getByTestId("artifact-card");
  await expect(cards).toHaveCount(5);
  for (const index of [0, 1, 2, 3, 4]) {
    await cards.nth(index).scrollIntoViewIfNeeded();
  }
  await expect(cards.nth(0)).toContainText("Revenue dashboard");
  await expect(cards.nth(0)).toContainText(
    "HTML · 18 KB · reports/quarterly-revenue-dashboard.html",
  );
  await expect(cards.nth(1)).toContainText("Revenue by region chart");
  await expect(cards.nth(1)).toContainText(
    "PNG · 68 B · charts/revenue-by-region.png",
  );
  await expect(cards.nth(2)).toContainText(
    "CSV · 1.5 MB · data/revenue_by_region_q3_2026_final_cleaned_export.csv",
  );
  await expect(cards.nth(3)).toContainText(
    "Markdown · 512 B · notes/analysis.md",
  );
  await expect(cards.nth(4)).toContainText(
    "File not found · missing/deleted-report.pdf",
  );

  await expect(
    page
      .frameLocator('iframe[title="Preview of Revenue dashboard"]')
      .getByText("Quarterly revenue"),
  ).toBeVisible();
  await expect(
    cards.nth(1).getByTestId("artifact-preview-image"),
  ).toHaveAttribute("src", `data:image/png;base64,${PNG_BASE64}`);
  await expect(
    cards.nth(2).getByTestId("artifact-preview-table"),
  ).toContainText("revenue");
  await expect(cards.nth(3).getByTestId("artifact-preview-text")).toContainText(
    "outperformed",
  );

  const chip = page.getByTestId("artifact-chip");
  await expect(chip).toHaveCount(1);
  await expect(chip).toHaveText("summary.txt");
  await expect(chip.locator("xpath=..")).toContainText(
    "The short summary.txt file sums it up in one paragraph.",
  );

  const missing = page.getByRole("button", {
    name: "Open Deleted report",
    exact: true,
  });
  await expect(missing).toBeDisabled();
  await expect(cards.nth(4)).toHaveAttribute("data-state", "missing");

  const htmlPath = `${ARTIFACTS_DIR}/reports/quarterly-revenue-dashboard.html`;
  const readsBeforeOpen = reads.length;
  await page
    .getByRole("button", { name: "Open Revenue dashboard", exact: true })
    .click();
  await expect.poll(() => reads.slice(readsBeforeOpen)).toContain(htmlPath);
  await expect(
    page.getByRole("tab", { name: /quarterly-revenue-dashboard\.html/ }),
  ).toBeVisible();
  expect(reads.every((path) => path.startsWith(`${ARTIFACTS_DIR}/`))).toBe(
    true,
  );
});
