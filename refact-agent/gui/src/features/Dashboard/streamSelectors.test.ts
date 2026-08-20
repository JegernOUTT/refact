import { describe, expect, it } from "vitest";
import type { RootState } from "../../app/store";
import type { ChatHistoryItem } from "../History/historySlice";
import type { TaskMeta } from "../../services/refact/tasks";
import {
  selectAttentionItems,
  selectContinueItems,
  selectFamilyChildren,
  selectStreamGroups,
  selectTodayAggregate,
  type StreamFilter,
  type StreamGroup,
  type StreamItem,
} from "./streamSelectors";

function atLocalNoon(dayOffset: number): string {
  const now = new Date();
  const date = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() - dayOffset,
    12,
    0,
    0,
    0,
  );
  return date.toISOString();
}

const TODAY = atLocalNoon(0);
const YESTERDAY = atLocalNoon(1);
const EARLIER = atLocalNoon(5);

function laterToday(minutes: number): string {
  return new Date(Date.parse(TODAY) + minutes * 60_000).toISOString();
}

function makeChat(partial: Partial<ChatHistoryItem> & { id: string }) {
  const chat: ChatHistoryItem = {
    title: partial.title ?? `chat ${partial.id}`,
    model: "gpt-5",
    mode: "AGENT",
    tool_use: "agent",
    messages: [],
    boost_reasoning: false,
    include_project_info: true,
    increase_max_tokens: false,
    isTitleGenerated: false,
    createdAt: partial.createdAt ?? TODAY,
    updatedAt: partial.updatedAt ?? TODAY,
    last_user_message_id: "",
    session_state: "idle",
    message_count: 1,
    ...partial,
  } as ChatHistoryItem;
  return chat;
}

function makeTask(partial: Partial<TaskMeta> & { id: string }): TaskMeta {
  return {
    name: partial.name ?? `task ${partial.id}`,
    status: "active",
    created_at: TODAY,
    updated_at: TODAY,
    cards_total: 0,
    cards_done: 0,
    cards_failed: 0,
    agents_active: 0,
    ...partial,
  };
}

function makeState(
  chats: ChatHistoryItem[],
  tasks: TaskMeta[] = [],
): RootState {
  const byId: Record<string, ChatHistoryItem> = {};
  for (const chat of chats) byId[chat.id] = chat;
  return {
    history: {
      chats: byId,
      isLoading: false,
      loadError: null,
      pagination: {
        cursor: null,
        hasMore: false,
        totalCount: chats.length,
        generation: 1,
      },
    },
    tasksApi: {
      queries: {
        "listTasks(undefined)": {
          endpointName: "listTasks",
          status: "fulfilled",
          data: tasks,
        },
      },
    },
  } as unknown as RootState;
}

const ALL: StreamFilter = { kind: "all", query: "" };

function flatten(groups: StreamGroup[]): string[] {
  return groups.flatMap((group) => group.items.map((item) => item.id));
}

function findItem(groups: StreamGroup[], id: string): StreamItem | undefined {
  for (const group of groups) {
    const found = group.items.find((item) => item.id === id);
    if (found) return found;
  }
  return undefined;
}

describe("selectStreamGroups — family collapse", () => {
  const family = [
    makeChat({ id: "root", title: "Root chat", updatedAt: EARLIER }),
    makeChat({
      id: "child",
      title: "Child refactor",
      parent_id: "root",
      updatedAt: laterToday(30),
    }),
    makeChat({
      id: "grand",
      title: "Grandchild sub agent",
      parent_id: "child",
      link_type: "subchat",
      updatedAt: YESTERDAY,
    }),
    makeChat({ id: "solo", title: "Solo chat", updatedAt: YESTERDAY }),
  ];

  it("shows one row per family and counts descendants", () => {
    const groups = selectStreamGroups(makeState(family), ALL);
    const ids = flatten(groups);

    expect(ids).toContain("root");
    expect(ids).not.toContain("child");
    expect(ids).not.toContain("grand");
    expect(ids).toContain("solo");

    const rootItem = findItem(groups, "root");
    expect(rootItem?.familyChildCount).toBe(2);
  });

  it("sorts a family by the most recent activity across the family", () => {
    const groups = selectStreamGroups(makeState(family), ALL);
    const rootItem = findItem(groups, "root");

    expect(rootItem?.updatedAtMs).toBe(Date.parse(laterToday(30)));

    const todayGroup = groups.find((group) => group.label === "Today");
    expect(todayGroup?.items[0].id).toBe("root");
  });

  it("puts solo chats in their own date group", () => {
    const groups = selectStreamGroups(makeState(family), ALL);
    const yesterday = groups.find((group) => group.label === "Yesterday");
    expect(yesterday?.items.map((item) => item.id)).toEqual(["solo"]);
  });
});

describe("selectStreamGroups — filtering", () => {
  const state = makeState(
    [
      makeChat({ id: "root", title: "Root chat", updatedAt: TODAY }),
      makeChat({
        id: "child",
        title: "Fix flaky websocket test",
        parent_id: "root",
        updatedAt: TODAY,
      }),
      makeChat({ id: "other", title: "Unrelated notes", updatedAt: TODAY }),
    ],
    [makeTask({ id: "task-1", name: "Websocket rewrite" })],
  );

  it("includes a family when the query matches a child title", () => {
    const groups = selectStreamGroups(state, {
      kind: "all",
      query: "websocket",
    });
    const ids = flatten(groups);
    expect(ids).toContain("root");
    expect(ids).toContain("task-1");
    expect(ids).not.toContain("other");
  });

  it("matches case-insensitively on the root title", () => {
    const groups = selectStreamGroups(state, { kind: "all", query: "ROOT" });
    expect(flatten(groups)).toEqual(["root"]);
  });

  it("scopes by kind", () => {
    const chatsOnly = selectStreamGroups(state, { kind: "chat", query: "" });
    expect(flatten(chatsOnly)).not.toContain("task-1");

    const tasksOnly = selectStreamGroups(state, { kind: "task", query: "" });
    expect(flatten(tasksOnly)).toEqual(["task-1"]);
  });
});

describe("selectStreamGroups — Active now", () => {
  it("groups streaming chats, working chats and busy tasks first", () => {
    const state = makeState(
      [
        makeChat({
          id: "streaming",
          session_state: "generating",
          updatedAt: EARLIER,
        }),
        makeChat({
          id: "working",
          session_state: "executing_tools",
          updatedAt: EARLIER,
        }),
        makeChat({ id: "idle", session_state: "idle", updatedAt: TODAY }),
      ],
      [
        makeTask({ id: "busy-task", agents_active: 2, updated_at: EARLIER }),
        makeTask({ id: "calm-task", updated_at: YESTERDAY }),
      ],
    );

    const groups = selectStreamGroups(state, ALL);
    expect(groups[0].label).toBe("Active now");
    expect(groups[0].items.map((item) => item.id).sort()).toEqual([
      "busy-task",
      "streaming",
      "working",
    ]);

    const labels = groups.map((group) => group.label);
    expect(labels).toEqual(["Active now", "Today", "Yesterday"]);
    expect(groups[1].items.map((item) => item.id)).toEqual(["idle"]);
  });
});

describe("selectContinueItems", () => {
  it("returns streaming/working items first, padded with recent ones", () => {
    const state = makeState(
      [
        makeChat({
          id: "streaming",
          session_state: "generating",
          updatedAt: EARLIER,
        }),
        makeChat({ id: "recent", updatedAt: laterToday(60) }),
        makeChat({ id: "older", updatedAt: laterToday(10) }),
        makeChat({ id: "oldest", updatedAt: YESTERDAY }),
      ],
      [],
    );

    const items = selectContinueItems(state);
    expect(items).toHaveLength(3);
    expect(items[0].id).toBe("streaming");
    expect(items.map((item) => item.id)).toEqual([
      "streaming",
      "recent",
      "older",
    ]);
  });

  it("is empty for an empty store", () => {
    expect(selectContinueItems(makeState([]))).toEqual([]);
  });
});

describe("selectAttentionItems", () => {
  it("picks tasks with failed cards plus failed/paused streams", () => {
    const state = makeState(
      [
        makeChat({
          id: "failed-chat",
          session_state: "error",
          updatedAt: laterToday(20),
        }),
        makeChat({
          id: "paused-chat",
          session_state: "paused",
          updatedAt: laterToday(10),
        }),
        makeChat({ id: "fine-chat", session_state: "idle" }),
      ],
      [
        makeTask({
          id: "failing-task",
          cards_failed: 2,
          cards_total: 5,
          cards_done: 1,
          updated_at: laterToday(30),
        }),
        makeTask({ id: "healthy-task" }),
      ],
    );

    const items = selectAttentionItems(state);
    expect(items.map((item) => item.id)).toEqual([
      "failing-task",
      "failed-chat",
      "paused-chat",
    ]);
    expect(items[0].cardsFailed).toBe(2);
    expect(items[0].cardsTotal).toBe(5);
    expect(items[0].cardsDone).toBe(1);
  });
});

describe("selectTodayAggregate", () => {
  it("sums chats updated today only", () => {
    const state = makeState([
      makeChat({
        id: "a",
        updatedAt: TODAY,
        total_cost_usd: 0.5,
        total_tokens: 100,
      }),
      makeChat({
        id: "b",
        updatedAt: laterToday(90),
        total_cost_usd: 0.25,
        total_tokens: 50,
      }),
      makeChat({
        id: "c",
        updatedAt: YESTERDAY,
        total_cost_usd: 10,
        total_tokens: 9999,
      }),
      makeChat({ id: "d", updatedAt: TODAY }),
    ]);

    expect(selectTodayAggregate(state)).toEqual({
      chats: 3,
      costUsd: 0.75,
      tokens: 150,
    });
  });
});

describe("selectFamilyChildren", () => {
  const state = makeState([
    makeChat({ id: "root", title: "Root chat" }),
    makeChat({
      id: "child",
      title: "Child chat",
      parent_id: "root",
      message_count: 4,
    }),
    makeChat({
      id: "grand",
      title: "Sub agent",
      parent_id: "child",
      link_type: "subchat",
      session_state: "generating",
      message_count: 7,
    }),
  ]);

  it("flattens descendants depth-first with 1-based depth and link types", () => {
    const children = selectFamilyChildren(state, "root");

    expect(children.map((child) => child.id)).toEqual(["child", "grand"]);
    expect(children[0].depth).toBe(1);
    expect(children[1].depth).toBe(2);
    expect(children[0].linkType).toBeNull();
    expect(children[1].linkType).toBe("subchat");
    expect(children[1].status).toBe("streaming");
    expect(children[1].messageCount).toBe(7);
  });

  it("is instance safe per root id", () => {
    const forRoot = selectFamilyChildren(state, "root");
    const forChild = selectFamilyChildren(state, "child");
    const forMissing = selectFamilyChildren(state, "nope");

    expect(forRoot.map((child) => child.id)).toEqual(["child", "grand"]);
    expect(forChild.map((child) => child.id)).toEqual(["grand"]);
    expect(forMissing).toEqual([]);
    expect(selectFamilyChildren(state, "root")).toBe(forRoot);
  });
});
