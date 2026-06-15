import { describe, expect, it } from "vitest";
import {
  CHAT_COMPANION_BUBBLE_GAP_MS,
  CHAT_COMPANION_OPEN_QUIET_MS,
  CHAT_COMPANION_STARTUP_QUIET_MS,
  deriveChatQuietUntil,
  gateChatCompanionBubble,
  initialChatQuietUntil,
  isChatCompanionWorthyRuntimeEvent,
  opportunityContentKey,
  runtimeEventContentKey,
  speechContentKey,
  suggestionContentKey,
  type ChatCompanionGateInput,
} from "../features/Buddy/buddyChatCompanionPolicy";
import type { BuddyRuntimeEvent } from "../features/Buddy/types";

function makeEvent(overrides?: Partial<BuddyRuntimeEvent>): BuddyRuntimeEvent {
  return {
    id: "evt-1",
    signal_type: "ordinary_status",
    title: "Runtime notice",
    source: "chat",
    status: "info",
    priority: "normal",
    created_at: "2024-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeGateInput(
  overrides?: Partial<ChatCompanionGateInput>,
): ChatCompanionGateInput {
  return {
    nowMs: 1_000_000,
    quietUntilMs: null,
    queuedMessageCount: 0,
    lastAmbientImpressionAtMs: null,
    candidateIsAmbient: true,
    candidateAlreadyImpressed: false,
    bypassGates: false,
    ...overrides,
  };
}

describe("isChatCompanionWorthyRuntimeEvent", () => {
  it("drops working and generating progress chatter even when persistent", () => {
    const noisyStatuses = [
      "started",
      "progress",
      "streaming",
      "running",
      "generating",
      "working",
      "queued",
      "completed",
    ];
    for (const status of noisyStatuses) {
      expect(
        isChatCompanionWorthyRuntimeEvent(
          makeEvent({
            signal_type: "workflow_progress",
            title: "Workflow running",
            status,
            persistent: true,
          }),
        ),
      ).toBe(false);
    }
  });

  it("drops progress chatter even when it carries controls", () => {
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({
          signal_type: "workflow_progress",
          status: "progress",
          controls: [
            { id: "stop", label: "Stop", action: "dismiss", style: "ghost" },
          ],
        }),
      ),
    ).toBe(false);
  });

  it("drops plain status notices without controls", () => {
    expect(
      isChatCompanionWorthyRuntimeEvent(makeEvent({ status: "info" })),
    ).toBe(false);
  });

  it("keeps humor, insight, and live chat reactions", () => {
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ signal_type: "speech_humor" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ signal_type: "speech_insight" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ source: "chat_reactions", status: "progress" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ signal_type: "chat_bug_candidate" }),
      ),
    ).toBe(true);
  });

  it("keeps errors, durable milestones, ambient policy, and actionable events", () => {
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ signal_type: "chat_error", status: "failed" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ signal_type: "quest_complete" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ bubble_policy: "ambient" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ bubble_policy: "durable" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({
          status: "needs_attention",
          controls: [
            { id: "go", label: "Go", action: "open_buddy", style: "primary" },
          ],
        }),
      ),
    ).toBe(true);
  });
});

describe("gateChatCompanionBubble", () => {
  it("allows the first bubble when the chat is calm", () => {
    expect(gateChatCompanionBubble(makeGateInput())).toEqual({
      allowed: true,
      reason: "shown",
      retryAtMs: null,
    });
  });

  it("mutes new bubbles while user messages are queued", () => {
    const verdict = gateChatCompanionBubble(
      makeGateInput({ queuedMessageCount: 2 }),
    );
    expect(verdict.allowed).toBe(false);
    expect(verdict.reason).toBe("queue_busy");
    expect(verdict.retryAtMs).toBeNull();
  });

  it("queued messages mute even urgent and already-shown bubbles", () => {
    expect(
      gateChatCompanionBubble(
        makeGateInput({ queuedMessageCount: 1, bypassGates: true }),
      ),
    ).toEqual({ allowed: false, reason: "queue_busy", retryAtMs: null });
    expect(
      gateChatCompanionBubble(
        makeGateInput({
          queuedMessageCount: 1,
          candidateAlreadyImpressed: true,
        }),
      ),
    ).toEqual({ allowed: false, reason: "queue_busy", retryAtMs: null });
  });

  it("stays quiet during the startup minute and retries when it ends", () => {
    const quietUntilMs = 1_030_000;
    const verdict = gateChatCompanionBubble(makeGateInput({ quietUntilMs }));
    expect(verdict.allowed).toBe(false);
    expect(verdict.reason).toBe("startup_quiet");
    expect(verdict.retryAtMs).toBe(quietUntilMs);

    expect(
      gateChatCompanionBubble(makeGateInput({ quietUntilMs: 999_999 })).allowed,
    ).toBe(true);
  });

  it("enforces the ambient cooldown gap only for never-shown ambient bubbles", () => {
    const lastAmbientImpressionAtMs = 1_000_000 - 10_000;
    const gated = gateChatCompanionBubble(
      makeGateInput({ lastAmbientImpressionAtMs }),
    );
    expect(gated.allowed).toBe(false);
    expect(gated.reason).toBe("cooldown");
    expect(gated.retryAtMs).toBe(
      lastAmbientImpressionAtMs + CHAT_COMPANION_BUBBLE_GAP_MS,
    );

    expect(
      gateChatCompanionBubble(
        makeGateInput({
          lastAmbientImpressionAtMs,
          candidateAlreadyImpressed: true,
        }),
      ).allowed,
    ).toBe(true);

    expect(
      gateChatCompanionBubble(
        makeGateInput({
          lastAmbientImpressionAtMs,
          candidateIsAmbient: false,
        }),
      ).allowed,
    ).toBe(true);

    expect(
      gateChatCompanionBubble(
        makeGateInput({
          lastAmbientImpressionAtMs: 1_000_000 - CHAT_COMPANION_BUBBLE_GAP_MS,
        }),
      ).allowed,
    ).toBe(true);
  });

  it("lets fresh urgent errors through every gate", () => {
    const verdict = gateChatCompanionBubble(
      makeGateInput({
        bypassGates: true,
        quietUntilMs: 2_000_000,
        lastAmbientImpressionAtMs: 999_999,
      }),
    );
    expect(verdict).toEqual({
      allowed: true,
      reason: "shown",
      retryAtMs: null,
    });
  });
});

describe("deriveChatQuietUntil", () => {
  it("starts the quiet window when the first user message lands", () => {
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: null,
        hadUserMessages: false,
        hasUserMessages: true,
        nowMs: 5_000,
      }),
    ).toBe(5_000 + CHAT_COMPANION_STARTUP_QUIET_MS);
  });

  it("keeps existing windows and ignores chats with history", () => {
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: 70_000,
        hadUserMessages: true,
        hasUserMessages: true,
        nowMs: 90_000,
      }),
    ).toBe(70_000);
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: null,
        hadUserMessages: true,
        hasUserMessages: true,
        nowMs: 90_000,
      }),
    ).toBeNull();
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: null,
        hadUserMessages: false,
        hasUserMessages: false,
        nowMs: 90_000,
      }),
    ).toBeNull();
  });

  it("extends an existing window when the first user message lands inside it", () => {
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: 100_000,
        hadUserMessages: false,
        hasUserMessages: true,
        nowMs: 90_000,
      }),
    ).toBe(90_000 + CHAT_COMPANION_STARTUP_QUIET_MS);
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: 500_000,
        hadUserMessages: false,
        hasUserMessages: true,
        nowMs: 90_000,
      }),
    ).toBe(500_000);
  });
});

describe("initialChatQuietUntil", () => {
  it("starts the open-chat quiet window only for chats with history", () => {
    expect(
      initialChatQuietUntil({ hasUserMessages: true, nowMs: 10_000 }),
    ).toBe(10_000 + CHAT_COMPANION_OPEN_QUIET_MS);
    expect(
      initialChatQuietUntil({ hasUserMessages: false, nowMs: 10_000 }),
    ).toBeNull();
  });
});

describe("notification content keys", () => {
  it("derives stable speech keys from dedupe keys and skips ambient speech", () => {
    expect(speechContentKey({ dedupe_key: "quest_prompt_start_setup" })).toBe(
      "content:speech:quest_prompt_start_setup",
    );
    expect(speechContentKey({ dedupe_key: undefined })).toBeNull();
    expect(
      speechContentKey({ speech_intent: "humor", dedupe_key: "anything" }),
    ).toBeNull();
    expect(speechContentKey({ dedupe_key: "speech_chat_reaction" })).toBeNull();
  });

  it("derives runtime keys from dedupe keys with an error-text fallback", () => {
    expect(
      runtimeEventContentKey(
        makeEvent({ dedupe_key: "llm_error:overloaded", status: "failed" }),
        "LLM error: Overloaded",
      ),
    ).toBe("content:runtime:llm_error_overloaded");
    expect(
      runtimeEventContentKey(
        makeEvent({ signal_type: "chat_error", status: "failed" }),
        "LLM error:   Overloaded  ",
      ),
    ).toBe("content:runtime:error:llm error: overloaded");
    expect(
      runtimeEventContentKey(makeEvent({ status: "info" }), "Plain notice"),
    ).toBeNull();
    expect(
      runtimeEventContentKey(
        makeEvent({ signal_type: "speech_humor", dedupe_key: "stable" }),
        "ha",
      ),
    ).toBeNull();
  });

  it("derives suggestion and opportunity keys from stable fields", () => {
    expect(
      suggestionContentKey({
        suggestion_type: "quest_start_setup",
        title: "Warm up this workspace",
      }),
    ).toBe("content:suggestion:quest_start_setup:warm up this workspace");
    expect(
      opportunityContentKey({
        kind: "task_health",
        cooldown_key: "task_health:stuck:global",
      }),
    ).toBe("content:opportunity:task_health:task_health:stuck:global");
    expect(
      opportunityContentKey({ kind: "task_health", cooldown_key: "  " }),
    ).toBeNull();
  });
});
