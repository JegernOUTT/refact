import { describe, expect, it } from "vitest";
import { applyChatEvent } from "../features/Chat/Thread/actions";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  selectCurrentPlan,
  selectEventLog,
  selectPlanHistory,
  selectVisibleMessages,
} from "../features/Chat/Thread/selectors";
import type { Chat, ChatThreadRuntime } from "../features/Chat/Thread/types";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import type {
  ChatMessages,
  EventMessage,
  PlanMessage,
} from "../services/refact/types";

type SelectorRootState = Parameters<typeof selectVisibleMessages>[0];

const threadId = "hidden-role-chat";

function makeRuntime(messages: ChatMessages = []): ChatThreadRuntime {
  return {
    thread: {
      id: threadId,
      messages,
      title: "Hidden Role Chat",
      model: "gpt-4",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      boost_reasoning: false,
      increase_max_tokens: false,
      include_project_info: true,
      auto_enrichment_enabled: false,
    },
    streaming: false,
    waiting_for_response: false,
    prevent_send: false,
    error: null,
    queued_items: [],
    send_immediately: false,
    attached_images: [],
    attached_text_files: [],
    background_agents: {},
    confirmation: {
      pause: false,
      pause_reasons: [],
      status: {
        wasInteracted: false,
        confirmationStatus: true,
      },
    },
    snapshot_received: true,
    task_widget_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
  };
}

function makeState(messages: ChatMessages = []): Chat {
  return {
    current_thread_id: threadId,
    open_thread_ids: [threadId],
    threads: { [threadId]: makeRuntime(messages) },
    system_prompt: {},
    tool_use: "agent",
    sse_refresh_requested: null,
    stream_version: 0,
  };
}

function makeRootState(messages: ChatMessages): SelectorRootState {
  return { chat: makeState(messages) } as SelectorRootState;
}

function makeEventMessage(overrides: Partial<EventMessage> = {}): EventMessage {
  return {
    role: "event",
    content: "mode changed",
    subkind: "mode_switch",
    source: "test",
    ...overrides,
  };
}

function makePlanMessage(
  version: number,
  overrides: Partial<PlanMessage> = {},
): PlanMessage {
  return {
    role: "plan",
    content: `plan ${version}`,
    extra: {
      plan: {
        mode: "agent",
        version,
        created_at_ms: version * 1000,
      },
    },
    ...overrides,
  };
}

const eventOne = makeEventMessage({
  message_id: "event-1",
  content: "first event",
  subkind: "mode_switch",
});
const eventTwo = makeEventMessage({
  message_id: "event-2",
  content: "second event",
  subkind: "tool_decision",
});
const planOne = makePlanMessage(1, { message_id: "plan-1" });
const planTwo = makePlanMessage(3, { message_id: "plan-3" });
const planThree = makePlanMessage(2, { message_id: "plan-2" });

const mixedMessages: ChatMessages = [
  { role: "system", content: "system prompt", message_id: "system-1" },
  { role: "user", content: "visible user", message_id: "user-1" },
  eventOne,
  {
    role: "assistant",
    content: "visible assistant",
    message_id: "assistant-1",
  },
  planOne,
  eventTwo,
  planTwo,
  planThree,
];

function makeMessageAddedEvent(
  message: EventMessage | PlanMessage,
): ChatEventEnvelope {
  return {
    chat_id: threadId,
    seq: "1",
    type: "message_added",
    index: 0,
    message,
  };
}

describe("hidden chat roles", () => {
  it("selectVisibleMessages excludes event and plan", () => {
    const visible = selectVisibleMessages(
      makeRootState(mixedMessages),
      threadId,
    );

    expect(visible).toHaveLength(3);
    expect(visible.map((message) => message.role)).toEqual([
      "system",
      "user",
      "assistant",
    ]);
  });

  it("selectEventLog returns only event messages", () => {
    const events = selectEventLog(makeRootState(mixedMessages), threadId);

    expect(events).toEqual([eventOne, eventTwo]);
  });

  it("selectCurrentPlan returns highest-version plan", () => {
    const plan = selectCurrentPlan(makeRootState(mixedMessages), threadId);

    expect(plan).toEqual(planTwo);
  });

  it("selectPlanHistory returns plans desc by version", () => {
    const plans = selectPlanHistory(makeRootState(mixedMessages), threadId);

    expect(plans).toEqual([planTwo, planThree, planOne]);
  });

  it("reducer accepts MessageAdded for role=event", () => {
    const state = chatReducer(
      makeState(),
      applyChatEvent(makeMessageAddedEvent(eventOne)),
    );

    expect(state.threads[threadId]?.thread.messages).toEqual([eventOne]);
  });

  it("reducer accepts MessageAdded for role=plan", () => {
    const state = chatReducer(
      makeState(),
      applyChatEvent(makeMessageAddedEvent(planOne)),
    );

    expect(state.threads[threadId]?.thread.messages).toEqual([planOne]);
  });
});
