import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import { chatReducer } from "./reducer";
import {
  applyChatEvent,
  closeThread,
  createChatWithId,
  switchToThread,
  removeChatFromCache,
  updateOpenThread,
} from "./actions";
import {
  selectThreadIds,
  selectTabsDisplayData,
  selectBackgroundAgentPool,
  selectBackgroundAgentsTree,
  selectActiveBackgroundAgents,
} from "./selectors";
import type { Chat } from "./types";

const history = { chats: {} };
const notifications = { pendingByThread: {}, lastSeenByThread: {} };
const root = (chat: Chat) =>
  ({ chat, history, notifications }) as unknown as RootState;
const selectors = [
  selectThreadIds,
  selectTabsDisplayData,
  selectBackgroundAgentPool,
  selectBackgroundAgentsTree,
  selectActiveBackgroundAgents,
];
function setup(count = 2) {
  let chat = chatReducer(undefined, { type: "test/init" });
  for (let i = 0; i < count; i++) {
    const id = `chat-${i}`;
    chat = chatReducer(chat, createChatWithId({ id, title: id }));
    chat = chatReducer(
      chat,
      applyChatEvent({
        chat_id: id,
        seq: "1",
        type: "stream_started",
        message_id: "stream",
      }),
    );
  }
  return chat;
}
function counts(chat: Chat) {
  const state = root(chat);
  selectThreadIds(state);
  selectTabsDisplayData(state);
  selectBackgroundAgentPool(state);
  selectBackgroundAgentsTree(state, "chat-0");
  selectActiveBackgroundAgents(state, "chat-0");
  return selectors.map((selector) => selector.recomputations());
}

describe("structural metadata inputs", () => {
  it("preserves busy runtimes on ordinary close and reopens without losing content", () => {
    const before = setup();
    const ids = selectThreadIds(root(before));
    const closed = chatReducer(before, closeThread({ id: "chat-0" }));
    expect(closed.open_thread_ids).not.toContain("chat-0");
    expect(closed.threads["chat-0"]).toBe(before.threads["chat-0"]);
    expect(selectThreadIds(root(closed))).toBe(ids);
    const removed = chatReducer(closed, removeChatFromCache({ id: "chat-0" }));
    expect(removed.threads["chat-0"]).toBe(before.threads["chat-0"]);
    const reopened = chatReducer(removed, switchToThread({ id: "chat-0" }));
    expect(reopened.open_thread_ids).toContain("chat-0");
    const forced = chatReducer(
      reopened,
      closeThread({ id: "chat-0", force: true }),
    );
    expect(forced.threads["chat-0"]).toBeUndefined();
    expect(selectThreadIds(root(forced))).not.toContain("chat-0");
    expect(before.open_thread_ids).toContain("chat-0");
  });

  it("characterizes independent tab metadata invalidation", () => {
    let before = setup();
    before = chatReducer(
      before,
      applyChatEvent({
        chat_id: "chat-0",
        seq: "2",
        type: "runtime_updated",
        state: "idle",
      }),
    );
    const baseline = selectTabsDisplayData(root(before));
    for (const thread of [
      { title: "renamed" },
      { mode: "explore" },
      { is_task_chat: true },
      {
        buddy_meta: {
          is_buddy_chat: true,
          buddy_chat_kind: "conversation" as const,
          workflow_id: null,
        },
      },
    ]) {
      const after = chatReducer(
        before,
        updateOpenThread({
          id: "chat-0",
          thread: { ...thread, isTitleGenerated: false },
        }),
      );
      const tabs = selectTabsDisplayData(root(after));
      expect(tabs[0]).not.toBe(baseline[0]);
      expect(tabs[1]).toBe(baseline[1]);
      expect(before.threads["chat-0"]?.thread.title).toBe("chat-0");
    }
  });

  it("does not run five result functions for 1500 ordered content deltas", () => {
    let chat = setup(100);
    const warmed = counts(chat);
    const before = chat;
    for (let round = 0; round < 15; round++) {
      for (let i = 0; i < 100; i++) {
        chat = chatReducer(
          chat,
          applyChatEvent({
            chat_id: `chat-${i}`,
            seq: String(round + 2),
            type: "stream_delta",
            message_id: "stream",
            ops: [{ op: "append_content", text: `${round},` }],
          }),
        );
        counts(chat);
      }
    }
    expect(counts(chat).map((count, i) => count - warmed[i])).toEqual([
      0, 0, 0, 0, 0,
    ]);
    for (let i = 0; i < 100; i++) {
      expect(chat.threads[`chat-${i}`]?.thread.messages.at(-1)?.content).toBe(
        Array.from({ length: 15 }, (_, round) => `${round},`).join(""),
      );
      expect(before.threads[`chat-${i}`]?.thread.messages.at(-1)?.content).toBe(
        "",
      );
    }
  });
});

it("invalidates activity for tool arguments while ignoring ordinary text", () => {
  let chat = setup();
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "2",
      type: "runtime_updated",
      state: "executing_tools",
    }),
  );
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "3",
      type: "stream_delta",
      message_id: "stream",
      ops: [
        {
          op: "set_tool_calls",
          tool_calls: [
            {
              id: "wait",
              index: 0,
              function: {
                name: "wait",
                arguments: '{"target":{"kind":"agents"}}',
              },
            },
          ],
        },
      ],
    }),
  );
  expect(selectTabsDisplayData(root(chat))[0].activity).toBe("waiting_agents");
  const before = counts(chat);
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "4",
      type: "stream_delta",
      message_id: "stream",
      ops: [{ op: "append_content", text: "text" }],
    }),
  );
  expect(counts(chat)).toEqual(before);
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "5",
      type: "stream_delta",
      message_id: "stream",
      ops: [
        {
          op: "set_tool_calls",
          tool_calls: [
            {
              id: "wait",
              index: 0,
              function: {
                name: "wait",
                arguments: '{"target":{"kind":"duration"}}',
              },
            },
          ],
        },
      ],
    }),
  );
  expect(selectTabsDisplayData(root(chat))[0].activity).toBe("waiting_timer");
  const after = counts(chat);
  expect(after[1]).toBe(before[1] + 1);
  expect(after[0]).toBe(before[0]);
  expect(after.slice(2)).toEqual(before.slice(2));
});

it("isolates background map revisions, terminal ties and duplicate winner removal", () => {
  let chat = setup();
  const agent = {
    agent_id: "agent",
    parent_chat_id: "chat-0",
    child_chat_id: "child",
    kind: "subagent",
    status: "running",
    title: "Inspect",
    progress: null,
    step_count: 0,
    last_activity: null,
    target_files: [],
    edited_files: [],
    diff_summary: null,
    conflict_summary: null,
    result_summary: null,
    error: null,
    started_at: null,
    finished_at: null,
    change_seq: 2,
    model_type: null,
    current_tool: null,
    goal_summary: null,
    plan_present: false,
    worktree_branch: null,
    merge_status: null,
    pending_questions: 0,
    questions: [],
    tokens_used: 0,
    cost_usd: null,
  } satisfies import("../../../services/refact/types").BackgroundAgentSummary;
  const before = counts(chat);
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "2",
      type: "background_agent_updated",
      agent,
    }),
  );
  const populated = counts(chat);
  expect(populated.slice(0, 2)).toEqual(before.slice(0, 2));
  expect(populated.slice(2)).toEqual(before.slice(2).map((n) => n + 1));
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "3",
      type: "stream_delta",
      message_id: "stream",
      ops: [{ op: "append_content", text: "text" }],
    }),
  );
  expect(counts(chat)).toEqual(populated);
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-1",
      seq: "2",
      type: "background_agent_updated",
      agent: { ...agent, change_seq: 1, title: "older" },
    }),
  );
  expect(selectBackgroundAgentPool(root(chat)).agent.title).toBe("Inspect");
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "4",
      type: "background_agent_updated",
      agent: { ...agent, change_seq: 1, title: "stale" },
    }),
  );
  const stale = counts(chat);
  chat = chatReducer(
    chat,
    applyChatEvent({
      chat_id: "chat-0",
      seq: "5",
      type: "background_agent_updated",
      agent: { ...agent, status: "completed" },
    }),
  );
  expect(selectActiveBackgroundAgents(root(chat), "chat-0")).toEqual([]);
  expect(counts(chat)[2]).toBe(stale[2] + 1);
  chat = chatReducer(chat, closeThread({ id: "chat-0", force: true }));
  expect(selectBackgroundAgentPool(root(chat)).agent.title).toBe("older");
  expect(selectActiveBackgroundAgents(root(chat), "chat-0")).toHaveLength(1);
});

it("does no metadata work for unrelated or global-only actions and targets one runtime for title", async () => {
  const activity = await import("../../../utils/chatActivity");
  const { vi } = await import("vitest");
  const { saveTitle, setToolUse } = await import("./actions");
  const { setSseStatus } = await import("../../Connection");
  let chat = setup(100);
  const spy = vi.spyOn(activity, "getChatActivity");
  try {
    const metadata = chat.metadata;
    chat = chatReducer(chat, { type: "chat/unrelated" });
    chat = chatReducer(chat, { type: "notifications/received" });
    chat = chatReducer(chat, setToolUse("quick"));
    chat = chatReducer(
      chat,
      setSseStatus({ chatId: "chat-0", status: "connected" }),
    );
    expect(chat.metadata).toBe(metadata);
    expect(spy).not.toHaveBeenCalled();
    chat = chatReducer(
      chat,
      saveTitle({ id: "chat-50", title: "new", isTitleGenerated: false }),
    );
    expect(spy).toHaveBeenCalledTimes(1);
    expect(chat.metadata?.display["chat-50"]?.title).toBe("new");
  } finally {
    spy.mockRestore();
  }
});

it("tracks the generated runtime id rather than an ignored new-chat payload id", async () => {
  const { newChatAction } = await import("./actions");
  const chat = chatReducer(
    setup(),
    newChatAction({ id: "ignored", title: "new" }),
  );
  expect(chat.current_thread_id).not.toBe("ignored");
  expect(selectThreadIds(root(chat))).toContain(chat.current_thread_id);
  expect(chat.metadata?.display[chat.current_thread_id]?.title).toBe("new");
});

it("keeps neutral actions safe for legacy partial runtimes without metadata", async () => {
  const actions = await import("./actions");
  const connection = await import("../../Connection");
  const chat = {
    threads: { x: {} },
    current_thread_id: "x",
  } as unknown as Chat;
  const neutral = [
    actions.setToolUse("agent"),
    actions.setAreFollowUpsEnabled(true),
    actions.setSystemPrompt({}),
    connection.setSseStatus({ chatId: "x", status: "connected" }),
    connection.removeSseConnection({ chatId: "x" }),
    connection.clearAllSseConnections(),
    actions.requestSseRefresh({ chatId: "x" }),
    actions.clearSseRefreshRequest(),
    actions.markQueuedCommandCancelled({ chatId: "x", clientRequestId: "q" }),
  ];
  for (const action of neutral) {
    const next = chatReducer(chat, action);
    expect(next.metadata).toBeUndefined();
  }
});

it("tracks generated integration runtime id rather than an extra ignored payload id", async () => {
  const { newIntegrationChat } = await import("./actions");
  const payload = {
    id: "chat-0",
    integration: { name: "test" },
    messages: [],
    request_attempt_id: "request",
  };
  const chat = chatReducer(setup(), newIntegrationChat(payload));
  expect(chat.current_thread_id).not.toBe("chat-0");
  expect(selectThreadIds(root(chat))).toContain(chat.current_thread_id);
  expect(chat.metadata?.display[chat.current_thread_id]).toBeDefined();
});
