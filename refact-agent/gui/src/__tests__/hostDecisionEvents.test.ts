import { beforeEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../app/store";
import { applyChatEvent } from "../features/Chat/Thread";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";
import {
  announceHostDecision,
  hostDecisionOutstanding,
  observeHostQuestionStatuses,
  resetHostDecisions,
  scopeHostDecisions,
} from "../events/hostDecisionTracker";
import { getEngineEndpointIdentity } from "../services/refact/apiUrl";

const question = {
  tool_call_id: "ask-1",
  asked_at_ms: 42,
  status: "pending" as const,
  questions: [{ id: "q", type: "yes_no" as const, text: "Proceed?" }],
};
const scope = {
  projectRoot: "/scratch",
  endpointIdentity: "http://localhost:8001",
};
const ask = {
  chatId: "chat-1",
  toolCallId: question.tool_call_id,
  questions: question.questions,
  askedAtMs: question.asked_at_ms,
  scope,
};

beforeEach(resetHostDecisions);

describe("native host decision producers", () => {
  it("correlates stream and chat announcements and observes terminal native status", () => {
    scopeHostDecisions(scope);
    expect(
      announceHostDecision(
        { ...ask, origin: "native_notification_stream" },
        scope,
      ),
    ).toBe(true);
    expect(
      announceHostDecision({ ...ask, origin: "native_chat_event" }, scope),
    ).toBe(false);
    expect(observeHostQuestionStatuses(scope, ask.chatId, undefined)).toEqual(
      [],
    );
    expect(observeHostQuestionStatuses(scope, ask.chatId, [])).toEqual([]);
    expect(observeHostQuestionStatuses(scope, ask.chatId, [question])).toEqual(
      [],
    );
    const resolved = observeHostQuestionStatuses(scope, ask.chatId, [
      { ...question, status: "answered" },
    ]);
    expect(resolved).toEqual([
      {
        chatId: ask.chatId,
        toolCallId: ask.toolCallId,
        askedAtMs: 42,
        scope,
        resolution: "answered",
        origin: "native_snapshot",
      },
    ]);
    expect(hostDecisionOutstanding()).toBe(0);
    expect(announceHostDecision(ask, scope)).toBe(false);
    expect(
      observeHostQuestionStatuses(scope, ask.chatId, [
        { ...question, status: "answered" },
      ]),
    ).toEqual([]);
  });

  it("rejects foreign endpoint and stale incarnation outcomes", () => {
    scopeHostDecisions(scope);
    announceHostDecision({ ...ask, askedAtMs: 43 }, scope);
    expect(
      observeHostQuestionStatuses(
        { ...scope, endpointIdentity: "foreign" },
        ask.chatId,
        [{ ...question, status: "cancelled" }],
      ),
    ).toEqual([]);
    expect(
      observeHostQuestionStatuses(scope, ask.chatId, [
        { ...question, status: "cancelled" },
      ]),
    ).toEqual([]);
    expect(hostDecisionOutstanding()).toBe(1);
    expect(
      observeHostQuestionStatuses(scope, ask.chatId, [
        { ...question, asked_at_ms: 43, status: "cancelled" },
      ]),
    ).toHaveLength(1);
    expect(announceHostDecision({ ...ask, askedAtMs: 42 }, scope)).toBe(false);
  });

  it("middleware emits answered without any mounted tool component and ignores omitted snapshots", () => {
    const postMessage = vi.fn();
    window.postIntellijMessage = postMessage;
    const store = setUpStore();
    const state = store.getState();
    const nativeScope = {
      projectRoot: state.current_project.workspaceRoots?.[0] ?? "",
      endpointIdentity: getEngineEndpointIdentity(state.config),
    };
    scopeHostDecisions(nativeScope);
    announceHostDecision({ ...ask, scope: nativeScope }, nativeScope);
    const snapshot: ChatEventEnvelope = {
      chat_id: ask.chatId,
      seq: "1",
      type: "snapshot",
      thread: {
        id: ask.chatId,
        title: "Test",
        model: "",
        mode: "AGENT",
        tool_use: "agent",
        boost_reasoning: false,
        context_tokens_cap: 8192,
        include_project_info: true,
        checkpoints_enabled: true,
        is_title_generated: false,
      },
      runtime: {
        state: "waiting_user_input",
        paused: false,
        error: null,
        queue_size: 0,
        pause_reasons: [],
        queued_items: [],
      },
      messages: [],
      background_agents: [],
    };
    store.dispatch(applyChatEvent(snapshot));
    expect(postMessage).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/decisionResolved" }),
    );
    store.dispatch(
      applyChatEvent({
        chat_id: ask.chatId,
        seq: "2",
        type: "message_added",
        index: 0,
        message: {
          role: "tool",
          tool_call_id: ask.toolCallId,
          content: JSON.stringify({
            type: "ask_questions",
            questions: question.questions,
            asked_at_ms: 42,
          }),
        },
      }),
    );
    // A sidebar announcement already exists: the actual chat emitter dedupes.
    expect(postMessage).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "ide/askQuestions" }),
    );
    store.dispatch(
      applyChatEvent({
        ...snapshot,
        seq: "3",
        pending_questions: [{ ...question, status: "answered" }],
      }),
    );
    expect(postMessage).toHaveBeenCalledWith({
      type: "ide/decisionResolved",
      payload: {
        chatId: ask.chatId,
        toolCallId: ask.toolCallId,
        askedAtMs: 42,
        scope: nativeScope,
        origin: "native_snapshot",
        resolution: "answered",
      },
    });
    delete window.postIntellijMessage;
  });
});
