import { expect, test } from "@playwright/test";
import type { setUpStore } from "../../src/app/store";

test("source chat file card sends an absolute path through the shipping read contract", async ({
  page,
}) => {
  const absolute = "/source-worktree/refact-agent/engine/src/caps/caps.rs";
  const reads: string[] = [];
  await page.route("**/v1/files/read?**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.searchParams.get("path") ?? "";
    reads.push(path);
    await route.fulfill({
      status: path === absolute ? 200 : 400,
      contentType: "application/json",
      body: JSON.stringify(
        path === absolute
          ? {
              path,
              content: "pub fn relative_link_contract() {}",
              language: "rust",
              size: 35,
              truncated: false,
              line_start: null,
              line_end: null,
              mtime_ms: 1,
            }
          : { detail: "Expected absolute path" },
      ),
    });
  });
  await page.addInitScript(() => {
    Object.assign(window, { __nativeFileLinkFetch: window.fetch.bind(window) });
  });
  await page.goto(
    "/tests/e2e/route-showcase.html?route=chat-dnd&toolbar_alignment&active=0",
  );
  await page.evaluate(() => {
    const fixtureFetch = window.fetch;
    const nativeFetch = (
      window as unknown as { __nativeFileLinkFetch: typeof fetch }
    ).__nativeFileLinkFetch;
    window.fetch = (input, init) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      return url.includes("/v1/files/read")
        ? nativeFetch(input, init)
        : fixtureFetch(input, init);
    };

    const store = (
      window as unknown as {
        __toolbarAlignmentStore: ReturnType<typeof setUpStore>;
      }
    ).__toolbarAlignmentStore;
    const chatId = "showcase-chat";
    const thread = store.getState().chat.threads[chatId]!.thread;
    store.dispatch({
      type: "chatThread/applyChatEvent",
      payload: {
        type: "snapshot",
        chat_id: chatId,
        seq: "1",
        thread: {
          ...thread,
          worktree: {
            id: "source",
            kind: "chat",
            root: "/source-worktree",
            source_workspace_root: "/project",
            repo_root: "/project",
            enforce: true,
          },
        },
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
            message_id: "file-user",
            content: "Inspect this file",
          },
          {
            role: "context_file",
            message_id: "file-context",
            content: [
              {
                file_name: "refact-agent/engine/src/caps/caps.rs",
                file_content: "pub fn preview() {}",
                line1: 1,
                line2: 2,
              },
            ],
          },
          {
            role: "assistant",
            message_id: "file-assistant",
            content: "The file is attached above.",
          },
        ],
        background_agents: [],
      },
    });
  });
  await page
    .getByRole("button", { name: "Context caps.rs:1-2 19 B", exact: true })
    .click();
  await page.getByRole("button", { name: "caps.rs:1-2", exact: true }).click();
  await expect.poll(() => reads).toContain(absolute);
  expect(reads.every((path) => path === absolute)).toBe(true);
  await expect(page.getByText("File unavailable", { exact: true })).toHaveCount(
    0,
  );
});
