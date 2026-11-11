import { expect, test, type Page, type Route } from "@playwright/test";

const taskId = "nav-task";
const plannerA = "nav-planner-a";
const plannerB = "nav-planner-b";
const agentChat = "nav-agent-chat";
const cardId = "C1";
const cardTitle = "Wire linked navigation";
const now = "2026-06-07T10:00:00Z";
const earlier = "2026-06-07T09:00:00Z";

const transcript: Record<string, string> = {
  [plannerA]: "Planner A transcript sentinel",
  [plannerB]: "Planner B transcript sentinel",
  [agentChat]: "Agent C1 transcript sentinel",
};
const titles: Record<string, string> = {
  [plannerA]: "Planner Alpha",
  [plannerB]: "Planner Beta",
  [agentChat]: `Agent: ${cardId} ${cardTitle}`,
};
const taskMeta: Record<string, Record<string, string>> = {
  [plannerA]: { task_id: taskId, role: "planner" },
  [plannerB]: { task_id: taskId, role: "planner" },
  [agentChat]: { task_id: taskId, role: "agents", card_id: cardId },
};

const task = {
  id: taskId,
  name: "Linked navigation task",
  status: "active",
  created_at: earlier,
  updated_at: now,
  cards_total: 1,
  cards_done: 0,
  cards_failed: 0,
  agents_active: 1,
};
const board = {
  schema_version: 1,
  rev: 1,
  columns: [
    { id: "planned", title: "Planned" },
    { id: "doing", title: "Doing" },
    { id: "done", title: "Done" },
  ],
  cards: [
    {
      id: cardId,
      title: cardTitle,
      column: "doing",
      priority: "P1",
      depends_on: [],
      instructions: "",
      assignee: "agent-1",
      agent_chat_id: agentChat,
      status_updates: [],
      final_report: null,
      created_at: earlier,
      started_at: earlier,
      completed_at: null,
      target_files: [],
    },
  ],
};
const planners = [
  {
    id: plannerA,
    title: titles[plannerA],
    created_at: earlier,
    updated_at: now,
  },
  {
    id: plannerB,
    title: titles[plannerB],
    created_at: earlier,
    updated_at: earlier,
  },
];

function snapshotEvent(chatId: string): string {
  const event = {
    type: "snapshot",
    seq: "1",
    chat_id: chatId,
    thread: {
      id: chatId,
      title: titles[chatId],
      model: "",
      mode: chatId === agentChat ? "TASK_AGENT" : "TASK_PLANNER",
      tool_use: "agent",
      boost_reasoning: false,
      context_tokens_cap: null,
      include_project_info: true,
      checkpoints_enabled: false,
      is_title_generated: true,
      task_meta: taskMeta[chatId],
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
      { role: "user", message_id: `${chatId}-u`, content: "Status?" },
      {
        role: "assistant",
        message_id: `${chatId}-a`,
        content: transcript[chatId],
      },
    ],
    background_agents: [],
  };
  const messages = event.messages;
  const bootstrap = [
    {
      ...event,
      messages: [],
      snapshot_chunked: true,
      chunk_count: 1,
      message_count: messages.length,
    },
    {
      type: "snapshot_chunk",
      chat_id: chatId,
      seq: event.seq,
      chunk_index: 0,
      chunk_count: 1,
      messages,
    },
    {
      type: "snapshot_end",
      chat_id: chatId,
      seq: event.seq,
      chunk_count: 1,
      message_count: messages.length,
    },
  ];
  return bootstrap.map((part) => `data: ${JSON.stringify(part)}\n\n`).join("");
}

type Mock = {
  requests: string[];
  fullHistory: string[];
  metadata: string[];
  subscribed: string[];
  holdMetadata: Set<string>;
  held: Route[];
};

async function mockEngine(page: Page): Promise<Mock> {
  const mock: Mock = {
    requests: [],
    fullHistory: [],
    metadata: [],
    subscribed: [],
    holdMetadata: new Set(),
    held: [],
  };
  await page.route(
    (url) => url.pathname.startsWith("/v1/"),
    async (route: Route) => {
      const request = route.request();
      const url = new URL(request.url());
      const path = url.pathname;
      const method = request.method();
      mock.requests.push(`${method} ${path}${url.search}`);
      const json = (body: unknown, status = 200) =>
        route.fulfill({ status, json: body });

      if (path === "/v1/chats/subscribe-many") {
        expect(url.searchParams.get("snapshot_chunks")).toBe("1");
        expect(url.searchParams.get("per_chat_close")).toBe("1");
        const chatIds: unknown = JSON.parse(
          url.searchParams.get("chat_ids") ?? "null",
        );
        if (
          !Array.isArray(chatIds) ||
          !chatIds.every((id: unknown): id is string => typeof id === "string")
        ) {
          return json({ detail: "invalid chat_ids" }, 400);
        }
        mock.subscribed.push(...chatIds);
        return route.fulfill({
          status: 200,
          headers: { "Content-Type": "text/event-stream" },
          body: chatIds
            .map((chatId: string) =>
              transcript[chatId]
                ? snapshotEvent(chatId)
                : `data: ${JSON.stringify({
                    type: "subscription_closed",
                    chat_id: chatId,
                    reason: "unavailable",
                    retryable: false,
                    message: "unknown chat",
                  })}\n\n`,
            )
            .join(""),
        });
      }
      const trajectory = /^\/v1\/trajectories\/([^/]+)$/.exec(path);
      if (trajectory && method === "GET") {
        const id = decodeURIComponent(trajectory[1]);
        if (url.searchParams.get("metadata_only") !== "true") {
          mock.fullHistory.push(id);
          return json({ detail: "full-history preflight refused" }, 500);
        }
        mock.metadata.push(id);
        if (mock.holdMetadata.has(id)) {
          mock.held.push(route);
          return;
        }
        if (!transcript[id]) return json({ detail: "not found" }, 404);
        return json({
          id,
          title: titles[id],
          model: "",
          mode: id === agentChat ? "TASK_AGENT" : "TASK_PLANNER",
          task_meta: taskMeta[id],
          ...(id === agentChat
            ? { parent_id: plannerA, link_type: "task_agent" }
            : {}),
        });
      }
      if (path === `/v1/tasks/${taskId}`) return json({ meta: task });
      if (path === `/v1/tasks/${taskId}/board`) return json(board);
      if (path === `/v1/tasks/${taskId}/trajectories/planner`)
        return json(planners);
      if (path === `/v1/tasks/${taskId}/trajectories/agents`)
        return json([
          {
            id: agentChat,
            title: titles[agentChat],
            created_at: earlier,
            updated_at: earlier,
            parent_id: plannerA,
          },
        ]);
      if (path === "/v1/worktrees")
        return json({
          project_hash: "linked-nav",
          source_workspace_root: "/workspace",
          source_current_branch: "main",
          source_branches: ["main"],
          worktrees: [],
        });
      if (path === "/v1/chat-modes") return json({ modes: [], errors: [] });
      return json({ detail: `unmocked ${path}` }, 404);
    },
  );
  return mock;
}

async function openFromSwitcher(page: Page, name: string) {
  await page.getByRole("button", { name: "Switch chat" }).click();
  await page.getByRole("button", { name, exact: true }).click();
}

const chatTab = (page: Page) => page.getByRole("tab", { name: "Chat" });
const transcriptOf = (page: Page, chatId: string) =>
  page.getByText(transcript[chatId], { exact: true });

test("task planner and subagent links open the Chat tab with the SSE snapshot transcript via metadata preflight only", async ({
  page,
}) => {
  const mock = await mockEngine(page);
  await page.goto(`/tests/e2e/linked-navigation.html?task=${taskId}`);

  await expect(chatTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(transcriptOf(page, plannerA)).toBeVisible();

  await openFromSwitcher(page, `Open chat ${titles[plannerB]}`);
  await expect(transcriptOf(page, plannerB)).toBeVisible();
  await expect(transcriptOf(page, plannerA)).toHaveCount(0);
  await expect(chatTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("alert")).toHaveCount(0);

  await page.getByRole("tab", { name: "Board" }).click();
  await expect(chatTab(page)).toHaveAttribute("aria-selected", "false");
  await page
    .getByRole("button", {
      name: `Open agent chat ${cardId} ${cardTitle}`,
      exact: true,
    })
    .first()
    .click();
  await expect(chatTab(page)).toHaveAttribute("aria-selected", "true");
  await expect(transcriptOf(page, agentChat)).toBeVisible();
  await expect(page.getByRole("button", { name: "Switch chat" })).toContainText(
    `Agent: ${cardId} ${cardTitle}`,
  );
  await expect(page.getByRole("alert")).toHaveCount(0);

  expect(mock.metadata).toEqual([plannerB, agentChat]);
  expect(mock.fullHistory).toEqual([]);
  expect(mock.subscribed).toEqual(
    expect.arrayContaining([plannerA, plannerB, agentChat]),
  );
});

test("a hung metadata preflight surfaces a timeout error and the next click still opens", async ({
  page,
}) => {
  const mock = await mockEngine(page);
  mock.holdMetadata.add(plannerB);
  await page.goto(`/tests/e2e/linked-navigation.html?task=${taskId}`);
  await expect(transcriptOf(page, plannerA)).toBeVisible();

  await page.clock.install();
  await openFromSwitcher(page, `Open chat ${titles[plannerB]}`);
  const pending = page
    .getByRole("status")
    .filter({ hasText: /Opening linked target(?:…|\.\.\.)/ });
  await expect.poll(() => mock.metadata).toEqual([plannerB]);
  // The task switcher is a trusted local target: retain the current transcript
  // without a global loading notice while metadata authorization is pending.
  await expect(pending).toHaveCount(0);
  await expect(transcriptOf(page, plannerA)).toBeVisible();

  await page.clock.fastForward(15_000);
  await expect(page.getByRole("alert")).toContainText("timed out");
  await expect(pending).toHaveCount(0);
  await expect(transcriptOf(page, plannerA)).toBeVisible();

  mock.holdMetadata.delete(plannerB);
  await openFromSwitcher(page, `Open chat ${titles[plannerB]}`);
  await expect(transcriptOf(page, plannerB)).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(mock.metadata).toEqual([plannerB, plannerB]);
  expect(mock.fullHistory).toEqual([]);
  await Promise.all(mock.held.splice(0).map((route) => route.abort()));
});
