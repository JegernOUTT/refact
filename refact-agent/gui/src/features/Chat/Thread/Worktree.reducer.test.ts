import { expect, test, describe } from "vitest";
import { chatReducer } from "./reducer";
import {
  applyChatEvent,
  buildThreadParamsPatch,
  createChatWithId,
} from "./actions";
import type { Chat, ChatThread } from "./types";
import type {
  ChatEventEnvelope,
  ThreadParams,
} from "../../../services/refact/chatSubscription";
import type { WorktreeMeta } from "../../../services/refact/worktrees";

function makeWorktreeMeta(id = "wt-shared"): WorktreeMeta {
  return {
    id,
    kind: "chat",
    root: `/tmp/${id}`,
    source_workspace_root: "/repo",
    repo_root: "/repo",
    branch: `refact/${id}`,
    base_branch: "main",
    base_commit: "abc123",
    enforce: false,
  };
}

function makeSnapshot(
  chatId: string,
  worktree?: WorktreeMeta | null,
  includeWorktree = true,
): ChatEventEnvelope {
  const thread: ThreadParams = {
    id: chatId,
    title: "Test",
    model: "gpt-4",
    mode: "AGENT",
    tool_use: "agent",
    boost_reasoning: false,
    context_tokens_cap: null,
    include_project_info: true,
    checkpoints_enabled: true,
    is_title_generated: false,
  };
  if (includeWorktree) {
    thread.worktree = worktree;
  }
  return {
    chat_id: chatId,
    seq: "1",
    type: "snapshot",
    thread,
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    background_agents: [],
    messages: [],
  };
}

function emptyChatState(): Chat {
  return chatReducer(undefined, { type: "@@INIT" });
}

describe("Worktree chat thread reducer", () => {
  test("snapshot stores worktree metadata", () => {
    const chatId = "chat-worktree-snapshot";
    const worktree = makeWorktreeMeta();

    const state = chatReducer(
      emptyChatState(),
      applyChatEvent(makeSnapshot(chatId, worktree)),
    );

    expect(state.threads[chatId]?.thread.worktree).toEqual(worktree);
  });

  test("thread_updated sets and detaches worktree metadata", () => {
    const chatId = "chat-worktree-update";
    const worktree = makeWorktreeMeta("wt-update");
    let state = chatReducer(
      emptyChatState(),
      applyChatEvent(makeSnapshot(chatId, undefined, false)),
    );

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "thread_updated",
        worktree,
      }),
    );

    expect(state.threads[chatId]?.thread.worktree).toEqual(worktree);

    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "3",
        type: "thread_updated",
        worktree: null,
      }),
    );

    expect(state.threads[chatId]?.thread.worktree).toBeNull();
  });

  test("old snapshot without worktree leaves worktree absent", () => {
    const chatId = "chat-old-snapshot";

    const state = chatReducer(
      emptyChatState(),
      applyChatEvent(makeSnapshot(chatId, undefined, false)),
    );

    expect(state.threads[chatId]?.thread.worktree).toBeUndefined();
  });

  test("snapshot without worktree preserves existing worktree metadata", () => {
    const chatId = "chat-preserve-worktree-snapshot";
    const worktree = makeWorktreeMeta("wt-preserve");
    let state = chatReducer(
      emptyChatState(),
      applyChatEvent(makeSnapshot(chatId, worktree)),
    );

    state = chatReducer(
      state,
      applyChatEvent({
        ...makeSnapshot(chatId, undefined, false),
        seq: "2",
      }),
    );

    expect(state.threads[chatId]?.thread.worktree).toEqual(worktree);
  });

  test("multiple threads can reference the same worktree id", () => {
    const sharedWorktree = makeWorktreeMeta("wt-shared");
    let state = emptyChatState();

    state = chatReducer(
      state,
      applyChatEvent(makeSnapshot("chat-a", sharedWorktree)),
    );
    state = chatReducer(
      state,
      applyChatEvent(makeSnapshot("chat-b", sharedWorktree)),
    );

    expect(state.threads["chat-a"]?.thread.worktree?.id).toBe("wt-shared");
    expect(state.threads["chat-b"]?.thread.worktree?.id).toBe("wt-shared");
    expect(state.threads["chat-a"]).toBeDefined();
    expect(state.threads["chat-b"]).toBeDefined();
  });

  test("createChatWithId accepts initial worktree metadata", () => {
    const worktree = makeWorktreeMeta("wt-initial");

    const state = chatReducer(
      emptyChatState(),
      createChatWithId({ id: "chat-initial", worktree }),
    );

    expect(state.threads["chat-initial"]?.thread.worktree).toEqual(worktree);
  });

  test("createChatWithId preserves source worktree for transition chats", () => {
    const worktree = makeWorktreeMeta("wt-transition");

    const state = chatReducer(
      emptyChatState(),
      createChatWithId({
        id: "chat-transition",
        parentId: "source-chat",
        linkType: "mode_transition",
        worktree,
      }),
    );

    expect(state.threads["chat-transition"]?.thread.worktree).toEqual(worktree);
    expect(state.threads["chat-transition"]?.thread.parent_id).toBe(
      "source-chat",
    );
    expect(state.threads["chat-transition"]?.thread.link_type).toBe(
      "mode_transition",
    );
  });

  test("thread params patch sends context token cap", () => {
    const thread: ChatThread = {
      id: "chat-context-cap",
      messages: [],
      model: "gpt-4",
      mode: "agent",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      context_tokens_cap: 128000,
    };

    expect(buildThreadParamsPatch(thread, true)).toHaveProperty(
      "context_tokens_cap",
      128000,
    );
  });

  test("thread params patch preserves worktree by id only", () => {
    const thread: ChatThread = {
      id: "chat-patch",
      messages: [],
      model: "gpt-4",
      mode: "agent",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      worktree: makeWorktreeMeta("wt-patch"),
    };

    const patch = buildThreadParamsPatch(thread, true);

    expect(patch).not.toHaveProperty("worktree");
    expect(patch).toHaveProperty("worktree_id", "wt-patch");
  });

  test("established thread params patch includes desired scope for changed-only sync", () => {
    const thread: ChatThread = {
      id: "chat-existing",
      messages: [],
      model: "gpt-4",
      mode: "agent",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      task_meta: {
        task_id: "task-1",
        role: "agent",
        card_id: "T-1",
      },
      worktree: makeWorktreeMeta("wt-existing"),
    };

    const patch = buildThreadParamsPatch(thread, false);

    expect(patch).toHaveProperty("model", "gpt-4");
    expect(patch).toHaveProperty("mode", "agent");
    expect(patch).toHaveProperty("tool_use", "agent");
    expect(patch).toHaveProperty("task_meta", thread.task_meta);
    expect(patch).toHaveProperty("worktree_id", "wt-existing");
  });
});
