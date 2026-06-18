import { describe, expect, it } from "vitest";
import { applyChatEvent } from "../features/Chat/Thread/actions";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  selectCurrentPlan,
  selectEventLog,
  selectGoalAttemptsById,
  selectGoalById,
  selectGoalContentById,
  selectGoalEventsById,
  selectGoalStatusById,
  selectPlanDeltaEvents,
  selectPlanHistory,
  selectSynthesizedPlanText,
  selectVisibleMessages,
} from "../features/Chat/Thread/selectors";
import type { Chat, ChatThreadRuntime } from "../features/Chat/Thread/types";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import type {
  ChatMessages,
  EventMessage,
  GoalMessage,
  GoalSnapshot,
  PlanMessage,
} from "../services/refact/types";

type SelectorRootState = Parameters<typeof selectVisibleMessages>[0];

const threadId = "hidden-role-chat";

const projectedGoal: GoalSnapshot = {
  content: "Projected goal",
  version: 2,
  active: true,
  status: "active",
  budget: {
    max_turns: 10,
    max_minutes: 15,
    max_tokens: 200000,
    cooldown_ms: 1500,
    no_progress_token_threshold: 50,
    no_progress_turns: 2,
  },
  progress: {
    turns_used: 1,
    tokens_used: 25,
    started_at_ms: 1000,
    no_progress_turns: 0,
    last_nudge_at_ms: 1500,
  },
  attempts: [
    {
      at_ms: 2000,
      trigger: "manual",
      verdict: "blocked",
      gaps: ["missing tests"],
      verifier_reply: "Add coverage",
    },
  ],
  events: [{ at_ms: 2500, kind: "started", text: "Goal started" }],
  transferred_from: null,
  transferred_to: null,
};

function makeRuntime(
  messages: ChatMessages = [],
  goal: GoalSnapshot | null = null,
): ChatThreadRuntime {
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
      goal,
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
    task_goal_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
  };
}

function makeState(
  messages: ChatMessages = [],
  goal: GoalSnapshot | null = null,
): Chat {
  return {
    current_thread_id: threadId,
    open_thread_ids: [threadId],
    threads: { [threadId]: makeRuntime(messages, goal) },
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

function makeBackendEventMessage(
  subkind: EventMessage["subkind"],
  content: string,
  payload: Record<string, unknown>,
  messageId: string,
): EventMessage {
  return {
    role: "event",
    content,
    message_id: messageId,
    extra: {
      event: {
        subkind,
        source: "tool.update_plan",
        payload,
      },
    },
  } as unknown as EventMessage;
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

function makeGoalMessage(overrides: Partial<GoalMessage> = {}): GoalMessage {
  return {
    role: "goal",
    content: "hidden goal body",
    message_id: "goal-1",
    extra: {
      goal: {
        mode: "agent",
        version: 1,
        created_at_ms: 1000,
        active: true,
        budget: projectedGoal.budget,
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
const planDeltaOne = makeEventMessage({
  message_id: "plan-delta-1",
  content: "first update",
  subkind: "plan_delta",
});
const planDeltaTwo = makeEventMessage({
  message_id: "plan-delta-2",
  content: "second update",
  subkind: "plan_delta",
});
const backendPlanDelta = makeBackendEventMessage(
  "plan_delta",
  "backend update",
  { seq: 1 },
  "backend-plan-delta",
);
const backendModeEvent = makeBackendEventMessage(
  "mode_switch",
  "backend mode event",
  { mode: "agent" },
  "backend-mode-event",
);
const goalDelta = makeEventMessage({
  message_id: "goal-delta-1",
  content: "goal update",
  subkind: "goal_delta",
});
const goalPursuit = makeEventMessage({
  message_id: "goal-pursuit-1",
  content: "goal pursuit",
  subkind: "goal_pursuit",
});
const planOne = makePlanMessage(1, { message_id: "plan-1" });
const planTwo = makePlanMessage(3, { message_id: "plan-3" });
const planThree = makePlanMessage(2, { message_id: "plan-2" });
const goalMessage = makeGoalMessage();

const mixedMessages: ChatMessages = [
  { role: "system", content: "system prompt", message_id: "system-1" },
  { role: "user", content: "visible user", message_id: "user-1" },
  eventOne,
  goalMessage,
  goalDelta,
  goalPursuit,
  {
    role: "assistant",
    content: "visible assistant",
    message_id: "assistant-1",
  },
  planOne,
  planDeltaOne,
  eventTwo,
  planTwo,
  planThree,
  backendPlanDelta,
  planDeltaTwo,
  backendModeEvent,
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
  it("selectVisibleMessages excludes event, plan, and goal roles", () => {
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

  it("selectEventLog returns only non-plan and non-goal event messages", () => {
    const events = selectEventLog(makeRootState(mixedMessages), threadId);

    expect(events.map((event) => event.message_id)).toEqual([
      "event-1",
      "event-2",
      "backend-mode-event",
    ]);
    expect(events.at(-1)).toMatchObject({
      subkind: "mode_switch",
      source: "tool.update_plan",
      payload: { mode: "agent" },
    });
  });

  it("selectGoal selectors read the thread projection", () => {
    const state = {
      chat: makeState(mixedMessages, projectedGoal),
    } as SelectorRootState;

    expect(selectGoalById(state, threadId)).toEqual(projectedGoal);
    expect(selectGoalStatusById(state, threadId)).toBe("active");
    expect(selectGoalContentById(state, threadId)).toBe("Projected goal");
    expect(selectGoalAttemptsById(state, threadId)).toEqual(
      projectedGoal.attempts,
    );
    expect(selectGoalEventsById(state, threadId)).toEqual(projectedGoal.events);
  });

  it("selectCurrentPlan returns highest-version plan", () => {
    const plan = selectCurrentPlan(makeRootState(mixedMessages), threadId);

    expect(plan).toEqual(planTwo);
  });

  it("selectPlanDeltaEvents returns plan_delta messages in index order", () => {
    const deltas = selectPlanDeltaEvents(
      makeRootState(mixedMessages),
      threadId,
    );

    expect(deltas.map((delta) => delta.message_id)).toEqual([
      "plan-delta-1",
      "backend-plan-delta",
      "plan-delta-2",
    ]);
    expect(deltas[1]).toMatchObject({
      subkind: "plan_delta",
      source: "tool.update_plan",
      payload: { seq: 1 },
    });
  });

  it("selectSynthesizedPlanText concatenates current base and deltas in order", () => {
    const text = selectSynthesizedPlanText(
      makeRootState(mixedMessages),
      threadId,
    );

    expect(text).toBe(
      "plan 3\n\n---\n\n## Plan updates\n\nfirst update\n\nbackend update\n\nsecond update",
    );
  });

  it("selectPlanHistory returns current base plus deltas", () => {
    const plans = selectPlanHistory(makeRootState(mixedMessages), threadId);

    expect(plans.map((plan) => plan.message_id)).toEqual([
      "plan-3",
      "plan-delta-1",
      "backend-plan-delta",
      "plan-delta-2",
    ]);
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
