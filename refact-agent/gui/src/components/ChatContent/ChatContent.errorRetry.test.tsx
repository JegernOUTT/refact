import { describe, expect, it } from "vitest";
import { within } from "@testing-library/react";
import { ChatContent } from "./ChatContent";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import type { ChatMessages } from "../../services/refact";
import type { RecoveryRecord } from "../../services/refact/chatSubscription";

function renderTranscript(messages: ChatMessages, recovery?: RecoveryRecord) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages = messages;
  runtime.snapshot_received = true;
  runtime.recovery = recovery;
  return render(
    <ChatContent
      onRetry={() => undefined}
      onStopStreaming={() => undefined}
      onRetryGeneration={() => undefined}
    />,
    { preloadedState: { chat } },
  );
}

describe("ChatContent transcript error retry", () => {
  it("offers a single Retry on the latest failure and none on history", async () => {
    renderTranscript([
      { role: "user", content: "Start", message_id: "u1" },
      { role: "error", content: "Old failure", message_id: "e1" },
      { role: "assistant", content: "Continued", message_id: "a1" },
      { role: "error", content: "First tail failure", message_id: "e2" },
      {
        role: "event",
        content: "Goal pursuit nudge",
        subkind: "system_notice",
        source: "chat.goal_monitor",
        message_id: "ev1",
      },
      { role: "error", content: "Second tail failure", message_id: "e3" },
    ]);

    const cards = await screen.findAllByTestId("error-message-card");
    expect(cards).toHaveLength(3);
    expect(screen.getAllByRole("button", { name: "Retry" })).toHaveLength(1);
    const latest = cards.find(
      (card) => card.textContent?.includes("Second tail failure"),
    );
    expect(latest).toBeDefined();
    if (!latest) return;
    expect(within(latest).getByRole("button", { name: "Retry" })).toBeVisible();
  });

  it("keeps Retry on a trailing compression failure", async () => {
    renderTranscript([
      { role: "user", content: "Start", message_id: "u1" },
      { role: "assistant", content: "Working", message_id: "a1" },
      {
        role: "event",
        content: "Context compression failed: summarizer unavailable",
        subkind: "system_notice",
        source: "chat.summarizer",
        message_id: "ev1",
      },
    ]);

    const card = await screen.findByTestId("error-message-card");
    expect(card).toHaveTextContent("Context compression failed");
    expect(within(card).getByRole("button", { name: "Retry" })).toBeVisible();
  });

  it("shows later failures after a recovered request instead of hiding them", async () => {
    const recovered: RecoveryRecord = {
      operation_id: "operation",
      run_id: "manual",
      revision: 3,
      state: "succeeded",
      policy: {
        mode: "transient",
        max_retries: 1,
        delay_ms: 10000,
        total_timeout_ms: 600000,
      },
      policy_source: "thread",
      manual: true,
      retries_used: 1,
      terminal_reason: "succeeded",
      latest_error: {
        category: "ProviderTransient",
        title: "Provider temporarily unavailable",
        explanation: "Temporary provider error.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: "LLM HTTP SSE response header timeout after 30s",
      },
      attempts: [],
    };
    renderTranscript(
      [
        { role: "user", content: "Start", message_id: "u1" },
        {
          role: "error",
          content: "Provider temporarily unavailable",
          message_id: "e1",
          extra: {
            recovery_record: {
              ...recovered,
              run_id: "automatic",
              state: "exhausted",
              manual: false,
            },
          },
        },
        { role: "assistant", content: "Recovered answer", message_id: "a1" },
        { role: "error", content: "Invalid active context", message_id: "e2" },
      ],
      recovered,
    );

    const card = await screen.findByTestId("error-message-card");
    expect(card).toHaveTextContent("Invalid active context");
    expect(card).not.toHaveTextContent("response header timeout");
    expect(within(card).getByRole("button", { name: "Retry" })).toBeVisible();
    expect(
      screen.queryByRole("region", { name: "Recovery status" }),
    ).toBeNull();
  });
});
