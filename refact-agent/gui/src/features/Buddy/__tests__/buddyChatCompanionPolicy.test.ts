import { describe, expect, it } from "vitest";
import {
  CHAT_COMPANION_BUBBLE_GAP_MS,
  CHAT_COMPANION_OPEN_QUIET_MS,
  CHAT_COMPANION_STARTUP_QUIET_MS,
  deriveChatQuietUntil,
  gateChatCompanionBubble,
  initialChatQuietUntil,
  isAmbientToken,
  isChatCompanionWorthyRuntimeEvent,
  isDurableSpeechToken,
  isLiveChatReactionEvent,
  isLiveChatReactionSignal,
  normalizedPolicyToken,
  opportunityContentKey,
  runtimeEventContentKey,
  speechContentKey,
  suggestionContentKey,
  type ChatCompanionGateInput,
} from "../buddyChatCompanionPolicy";
import type { BuddyRuntimeEvent } from "../types";

function makeEvent(overrides?: Partial<BuddyRuntimeEvent>): BuddyRuntimeEvent {
  return {
    id: "event-1",
    signal_type: "ordinary_status",
    title: "Runtime notice",
    source: "runtime",
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

describe("buddyChatCompanionPolicy", () => {
  it("normalizes policy tokens and detects ambient membership", () => {
    expect(normalizedPolicyToken(undefined)).toBe("");
    expect(normalizedPolicyToken(null)).toBe("");
    expect(normalizedPolicyToken("")).toBe("");
    expect(normalizedPolicyToken(" Speech-Humor:Now ")).toBe("humor_now");
    expect(normalizedPolicyToken("speech_chat_reaction")).toBe("chat_reaction");

    expect(isAmbientToken(undefined)).toBe(false);
    expect(isAmbientToken("  ")).toBe(false);
    expect(isAmbientToken("humor")).toBe(true);
    expect(isAmbientToken("speech_insight")).toBe(true);
    expect(isAmbientToken("memory_pulse_commentary")).toBe(true);
    expect(isAmbientToken("error_alert")).toBe(false);
  });

  it("detects durable speech tokens including error alerts", () => {
    expect(isDurableSpeechToken(undefined)).toBe(false);
    expect(isDurableSpeechToken("")).toBe(false);
    expect(isDurableSpeechToken("speech_quest_complete")).toBe(true);
    expect(isDurableSpeechToken("error-alert")).toBe(true);
    expect(isDurableSpeechToken("humor")).toBe(false);
  });

  it("detects live chat reaction signals and events", () => {
    expect(isLiveChatReactionSignal(undefined)).toBe(false);
    expect(isLiveChatReactionSignal("speech_chat_reaction")).toBe(true);
    expect(isLiveChatReactionSignal("chat-interaction-comment")).toBe(true);
    expect(isLiveChatReactionSignal("ordinary_status")).toBe(false);

    expect(
      isLiveChatReactionEvent(makeEvent({ source: "chat_reactions" })),
    ).toBe(true);
    expect(
      isLiveChatReactionEvent(makeEvent({ signal_type: "chat_bug_candidate" })),
    ).toBe(true);
    expect(
      isLiveChatReactionEvent(
        makeEvent({ dedupe_key: "speech_chat_reaction" }),
      ),
    ).toBe(true);
    expect(isLiveChatReactionEvent(makeEvent())).toBe(false);
  });

  it("classifies runtime events worthy of chat companion bubbles", () => {
    expect(
      isChatCompanionWorthyRuntimeEvent(makeEvent({ status: "running" })),
    ).toBe(false);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({
          status: "generating",
          controls: [
            { id: "stop", label: "Stop", action: "dismiss", style: "ghost" },
          ],
        }),
      ),
    ).toBe(false);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ status: "failed", signal_type: "chat_error" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({ source: "chat_reactions", status: "progress" }),
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
        makeEvent({ signal_type: "quest_complete" }),
      ),
    ).toBe(true);
    expect(
      isChatCompanionWorthyRuntimeEvent(
        makeEvent({
          controls: [
            {
              id: "open",
              label: "Open",
              action: "open_buddy",
              style: "primary",
            },
          ],
        }),
      ),
    ).toBe(true);
    expect(isChatCompanionWorthyRuntimeEvent(makeEvent())).toBe(false);
  });

  it("builds speech content keys only for non-ambient stable dedupe keys", () => {
    expect(speechContentKey({ dedupe_key: undefined })).toBeNull();
    expect(speechContentKey({ dedupe_key: "" })).toBeNull();
    expect(
      speechContentKey({ speech_intent: "humor", dedupe_key: "quest_start" }),
    ).toBeNull();
    expect(speechContentKey({ dedupe_key: "speech_chat_reaction" })).toBeNull();
    expect(
      speechContentKey({
        speech_intent: "error_alert",
        dedupe_key: "Quest Prompt-Start",
      }),
    ).toBe("content:speech:quest_prompt_start");
  });

  it("scopes speech content keys so different workspaces do not cross-suppress", () => {
    const firstWorkspaceKey = speechContentKey({
      dedupe_key: "Quest Prompt-Start",
      workspace_id: "workspace-a",
    });
    const secondWorkspaceKey = speechContentKey({
      dedupe_key: "Quest Prompt-Start",
      workspace_id: "workspace-b",
    });
    const seenNotificationIds: Record<string, number> = {};

    expect(firstWorkspaceKey).toBe(
      "scope:workspace-a:content:speech:quest_prompt_start",
    );
    expect(secondWorkspaceKey).toBe(
      "scope:workspace-b:content:speech:quest_prompt_start",
    );
    expect(firstWorkspaceKey).not.toBe(secondWorkspaceKey);

    if (firstWorkspaceKey != null) {
      seenNotificationIds[firstWorkspaceKey] = Date.now();
    }

    expect(
      secondWorkspaceKey != null && secondWorkspaceKey in seenNotificationIds,
    ).toBe(false);
  });

  it("keeps same-scope speech content keys stable so repeated content suppresses", () => {
    const firstKey = speechContentKey({
      dedupe_key: "Quest Prompt-Start",
      workspace_id: "workspace-a",
    });
    const repeatedKey = speechContentKey({
      dedupe_key: "Quest Prompt-Start",
      workspace_id: "workspace-a",
    });
    const seenNotificationIds: Record<string, number> = {};

    expect(firstKey).toBe(repeatedKey);

    if (firstKey != null) {
      seenNotificationIds[firstKey] = Date.now();
    }

    expect(repeatedKey != null && repeatedKey in seenNotificationIds).toBe(
      true,
    );
  });

  it("includes chat scope with workspace scope when available", () => {
    expect(
      speechContentKey({
        dedupe_key: "Quest Prompt-Start",
        workspace_id: "workspace-a",
        chat_id: "chat-1",
      }),
    ).toBe("scope:workspace-a:chat-1:content:speech:quest_prompt_start");
    expect(
      speechContentKey({
        dedupe_key: "Quest Prompt-Start",
        workspace_id: "workspace-a",
        chat_id: "chat-2",
      }),
    ).toBe("scope:workspace-a:chat-2:content:speech:quest_prompt_start");
  });

  it("builds runtime, suggestion, and opportunity content keys", () => {
    expect(
      runtimeEventContentKey(
        makeEvent({ signal_type: "speech_humor", dedupe_key: "stable" }),
        "ha",
      ),
    ).toBeNull();
    expect(
      runtimeEventContentKey(
        makeEvent({ source: "chat_reactions", dedupe_key: "stable" }),
        "ha",
      ),
    ).toBeNull();
    expect(
      runtimeEventContentKey(
        makeEvent({ dedupe_key: "LLM error:overloaded" }),
        "ignored",
      ),
    ).toBe("content:runtime:llm_error_overloaded");
    expect(
      runtimeEventContentKey(
        makeEvent({ signal_type: "chat_error", status: "failed" }),
        " LLM error:   Overloaded  ",
      ),
    ).toBe("content:runtime:error:llm error: overloaded");
    expect(
      runtimeEventContentKey(
        makeEvent({ signal_type: "chat_error", status: "failed" }),
        "   ",
      ),
    ).toBeNull();
    expect(runtimeEventContentKey(makeEvent(), "Plain notice")).toBeNull();

    expect(
      suggestionContentKey({
        suggestion_type: "Quest Start Setup",
        title: " Warm up   workspace ",
      }),
    ).toBe("content:suggestion:quest_start_setup:warm up workspace");
    expect(
      opportunityContentKey({
        kind: "task_health",
        cooldown_key: "task_health:stuck:global",
      }),
    ).toBe("content:opportunity:task_health:task_health:stuck:global");
    expect(
      opportunityContentKey({ kind: "task_health", cooldown_key: "  " }),
    ).toBeNull();
    expect(
      opportunityContentKey({ kind: "task_health", cooldown_key: null }),
    ).toBeNull();
  });

  it("gates chat companion bubbles for quiet windows, busy queues, cooldowns, and bypasses", () => {
    expect(gateChatCompanionBubble(makeGateInput())).toEqual({
      allowed: true,
      reason: "shown",
      retryAtMs: null,
    });
    expect(
      gateChatCompanionBubble(
        makeGateInput({ queuedMessageCount: 1, bypassGates: true }),
      ),
    ).toEqual({ allowed: false, reason: "queue_busy", retryAtMs: null });
    expect(
      gateChatCompanionBubble(makeGateInput({ quietUntilMs: 1_010_000 })),
    ).toEqual({
      allowed: false,
      reason: "startup_quiet",
      retryAtMs: 1_010_000,
    });
    expect(
      gateChatCompanionBubble(makeGateInput({ quietUntilMs: Number.NaN }))
        .allowed,
    ).toBe(true);
    expect(
      gateChatCompanionBubble(
        makeGateInput({ bypassGates: true, quietUntilMs: 1_010_000 }),
      ),
    ).toEqual({ allowed: true, reason: "shown", retryAtMs: null });
    expect(
      gateChatCompanionBubble(
        makeGateInput({
          candidateAlreadyImpressed: true,
          quietUntilMs: 1_010_000,
        }),
      ),
    ).toEqual({ allowed: true, reason: "shown", retryAtMs: null });

    const lastAmbientImpressionAtMs = 1_000_000 - 10_000;
    expect(
      gateChatCompanionBubble(makeGateInput({ lastAmbientImpressionAtMs })),
    ).toEqual({
      allowed: false,
      reason: "cooldown",
      retryAtMs: lastAmbientImpressionAtMs + CHAT_COMPANION_BUBBLE_GAP_MS,
    });
    expect(
      gateChatCompanionBubble(
        makeGateInput({ lastAmbientImpressionAtMs, candidateIsAmbient: false }),
      ).allowed,
    ).toBe(true);
    expect(
      gateChatCompanionBubble(
        makeGateInput({ lastAmbientImpressionAtMs: Number.NaN }),
      ).allowed,
    ).toBe(true);
  });

  it("derives quiet windows for startup and existing chat opens", () => {
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: null,
        hadUserMessages: false,
        hasUserMessages: true,
        nowMs: 5_000,
      }),
    ).toBe(5_000 + CHAT_COMPANION_STARTUP_QUIET_MS);
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: null,
        hadUserMessages: false,
        hasUserMessages: false,
        nowMs: 5_000,
      }),
    ).toBeNull();
    expect(
      deriveChatQuietUntil({
        previousQuietUntilMs: 100_000,
        hadUserMessages: true,
        hasUserMessages: true,
        nowMs: 5_000,
      }),
    ).toBe(100_000);
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

    expect(
      initialChatQuietUntil({ hasUserMessages: true, nowMs: 10_000 }),
    ).toBe(10_000 + CHAT_COMPANION_OPEN_QUIET_MS);
    expect(
      initialChatQuietUntil({ hasUserMessages: false, nowMs: 10_000 }),
    ).toBeNull();
  });
});
