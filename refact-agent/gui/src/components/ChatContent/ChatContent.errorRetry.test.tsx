import { describe, expect, it } from "vitest";
import { waitFor, within } from "@testing-library/react";
import { ChatContent } from "./ChatContent";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import type {
  ChatMessage,
  ChatMessages,
  ErrorMessage,
} from "../../services/refact";
import type {
  ChatEventEnvelope,
  RecoveryRecord,
} from "../../services/refact/chatSubscription";
import { applyChatEvent } from "../../features/Chat/Thread/actions";

const record: RecoveryRecord = {
  operation_id: "operation",
  run_id: "run",
  revision: 2,
  state: "exhausted",
  policy: {
    mode: "transient",
    max_retries: 2,
    delay_ms: 10000,
    total_timeout_ms: 600000,
  },
  policy_source: "thread",
  manual: false,
  retries_used: 1,
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

function attemptError(
  id: string,
  content: string,
  extra: Record<string, unknown> = {},
): ErrorMessage {
  return {
    role: "error",
    content,
    message_id: id,
    extra: {
      _ui_only: true,
      recovery_operation_id: record.operation_id,
      recovery_run_id: record.run_id,
      ...extra,
    },
  };
}

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

function expectEachShownOnce(texts: string[]) {
  const shown = document.body.textContent ?? "";
  texts.forEach((text) => {
    expect({ text, count: shown.split(text).length - 1 }).toEqual({
      text,
      count: 1,
    });
  });
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

    const cards = await screen.findAllByTestId("error-message-card");
    expect(cards).toHaveLength(2);
    expect(cards[0]).toHaveTextContent("response header timeout");
    expect(
      within(cards[0]).queryByRole("button", { name: "Retry" }),
    ).toBeNull();
    expect(cards[1]).toHaveTextContent("Invalid active context");
    expect(cards[1]).not.toHaveTextContent("response header timeout");
    expect(
      within(cards[1]).getByRole("button", { name: "Retry" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("region", { name: "Recovery status" }),
    ).toBeNull();
  });

  it("keeps recovered errors visible with a compact recovered status", async () => {
    const succeeded: RecoveryRecord = {
      ...record,
      state: "succeeded",
      retries_used: 2,
      terminal_reason: null,
    };
    renderTranscript(
      [
        { role: "user", content: "Start", message_id: "u1" },
        attemptError("e1", "First 429", { repeat_count: 3 }),
        attemptError("e2", "Second 429", { recovery_record: succeeded }),
        { role: "assistant", content: "Recovered answer", message_id: "a1" },
        { role: "user", content: "Next", message_id: "u2" },
      ],
      succeeded,
    );

    const card = await screen.findByTestId("error-message-card");
    expect(card).toHaveTextContent("First 429");
    expect(card).toHaveTextContent("Second 429");
    expect(within(card).getByTestId("error-repeat-count")).toHaveTextContent(
      "×3",
    );
    expect(within(card).getByTestId("error-recovery-status")).toHaveTextContent(
      "Recovered after 2 retries",
    );
    expect(within(card).queryByRole("button", { name: "Retry" })).toBeNull();
    expect(
      screen.queryByRole("region", { name: "Recovery status" }),
    ).toBeNull();
  });

  it("renders the live recovery card inline in place of its hosting error", async () => {
    const live: RecoveryRecord = {
      ...record,
      state: "waiting",
      latest_error: { ...record.latest_error, raw_error: "Latest attempt" },
      attempts: [
        {
          at_ms: 1,
          retry_number: 0,
          outcome: "failed",
          error: { ...record.latest_error, raw_error: "Earlier attempt" },
        },
      ],
    };
    const stamped: RecoveryRecord = {
      ...live,
      attempts: live.attempts.map((attempt) => ({ ...attempt, error: null })),
    };
    renderTranscript(
      [
        { role: "user", content: "Start", message_id: "u1" },
        attemptError("e1", "Earlier attempt"),
        {
          role: "event",
          content: "Goal pursuit nudge",
          subkind: "system_notice",
          source: "chat.goal_monitor",
          message_id: "ev1",
        },
        attemptError("b1", "Other operation error", {
          recovery_operation_id: "other-operation",
          recovery_run_id: "other-run",
        }),
        attemptError("e2", "Latest attempt", { recovery_record: stamped }),
        attemptError("g1", "Later other error", {
          recovery_operation_id: "other-operation",
          recovery_run_id: "other-run",
        }),
      ],
      live,
    );

    const panels = await screen.findAllByRole("region", {
      name: "Recovery status",
    });
    expect(panels).toHaveLength(1);
    const panel = panels[0];
    expect(panel).toHaveTextContent("Latest attempt");
    expect(panel).not.toHaveTextContent("Earlier attempt");
    expect(panel.closest("[data-testid='chat-virtuoso-item']")).not.toBeNull();
    expect(panel.closest("[data-testid='chat-virtuoso-item']")).toHaveAttribute(
      "data-chat-scroll-anchor-key",
      "b1",
    );
    const cards = screen.getAllByTestId("error-message-card");
    expect(cards.map((card) => card.textContent)).toEqual([
      expect.stringContaining("Earlier attempt"),
      expect.stringContaining("Other operation error"),
      expect.stringContaining("Later other error"),
    ]);
    const order = [cards[0], cards[1], panel, cards[2]];
    order.slice(1).forEach((node, index) => {
      expect(
        order[index].compareDocumentPosition(node) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    });
    expectEachShownOnce([
      "Earlier attempt",
      "Latest attempt",
      "Other operation error",
      "Later other error",
    ]);
    expect(screen.queryByTestId("error-recovery-status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("shows each diagnostic of an episode split across groups exactly once, live and after success", async () => {
    const attempts: RecoveryRecord["attempts"] = [
      {
        at_ms: 1,
        retry_number: 0,
        outcome: "failed",
        error: { ...record.latest_error, raw_error: "First cause" },
      },
      {
        at_ms: 2,
        retry_number: 1,
        outcome: "failed",
        error: { ...record.latest_error, raw_error: "Second cause" },
      },
    ];
    const live: RecoveryRecord = {
      ...record,
      state: "waiting",
      retries_used: 1,
      latest_error: { ...record.latest_error, raw_error: "Second cause" },
      attempts,
    };
    const transcript = (stamp: RecoveryRecord): ChatMessages => [
      { role: "user", content: "Start", message_id: "u1" },
      attemptError("e1", "First cause"),
      {
        role: "assistant",
        content: "Partial output",
        message_id: "p1",
        extra: { _ui_only: true, interrupted: true },
      },
      attemptError("e2", "Second cause", { recovery_record: stamp }),
    ];
    const view = renderTranscript(transcript(live), live);

    const panel = await screen.findByRole("region", {
      name: "Recovery status",
    });
    expect(panel).toHaveTextContent("Second cause");
    expect(panel).not.toHaveTextContent("First cause");
    expectEachShownOnce(["First cause", "Second cause"]);

    const succeeded: RecoveryRecord = {
      ...live,
      state: "succeeded",
      retries_used: 2,
    };
    view.unmount();
    renderTranscript(
      [
        ...transcript(succeeded),
        { role: "assistant", content: "Recovered answer", message_id: "a1" },
      ],
      succeeded,
    );
    const cards = await screen.findAllByTestId("error-message-card");
    expect(cards).toHaveLength(2);
    expect(
      screen.queryByRole("region", { name: "Recovery status" }),
    ).toBeNull();
    expectEachShownOnce(["First cause", "Second cause"]);
    expect(
      within(cards[1]).getByTestId("error-recovery-status"),
    ).toHaveTextContent("Recovered after 2 retries");
  });

  it("expands the stamped history of a legacy title-only archived error", async () => {
    const exhausted: RecoveryRecord = {
      ...record,
      attempts: [
        {
          at_ms: 1,
          retry_number: 0,
          outcome: "failed",
          error: { ...record.latest_error, raw_error: "Legacy first cause" },
        },
      ],
    };
    renderTranscript([
      { role: "user", content: "Start", message_id: "u1" },
      {
        role: "error",
        content: record.latest_error.title,
        message_id: "e1",
        extra: { recovery_record: exhausted },
      },
    ]);
    const card = await screen.findByTestId("error-message-card");
    expect(card).toHaveTextContent("Legacy first cause");
    expect(card).toHaveTextContent("response header timeout");
    expectEachShownOnce(["Legacy first cause", "response header timeout"]);
  });

  it("falls back to the footer card when no transcript error hosts the live record", async () => {
    const live: RecoveryRecord = {
      ...record,
      run_id: "manual",
      operation_id: "other",
      manual: true,
      state: "attempting",
    };
    renderTranscript(
      [
        { role: "user", content: "Start", message_id: "u1" },
        { role: "assistant", content: "Answer", message_id: "a1" },
      ],
      live,
    );

    const panels = await screen.findAllByRole("region", {
      name: "Recovery status",
    });
    expect(panels).toHaveLength(1);
    expect(panels[0].closest("[data-testid='chat-virtuoso-item']")).toBeNull();
    expect(panels[0]).toHaveTextContent("response header timeout");
  });

  it("shows a stale non-terminal stamped record as interrupted", async () => {
    renderTranscript([
      { role: "user", content: "Start", message_id: "u1" },
      attemptError("e1", "Stale attempt", {
        recovery_record: { ...record, state: "waiting" },
      }),
    ]);

    const card = await screen.findByTestId("error-message-card");
    expect(within(card).getByTestId("error-recovery-status")).toHaveTextContent(
      "Interrupted",
    );
    expect(
      screen.queryByRole("region", { name: "Recovery status" }),
    ).toBeNull();
  });
});

describe("ChatContent live recovery driven by chat events", () => {
  it("updates the inline live card and leaves a static recovered card in place", async () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].thread.messages = [
      { role: "user", content: "Start", message_id: "u1" },
    ];
    const view = render(
      <ChatContent
        onRetry={() => undefined}
        onStopStreaming={() => undefined}
        onRetryGeneration={() => undefined}
      />,
      { preloadedState: { chat } },
    );
    let seq = 0;
    const apply = (event: Record<string, unknown>) => {
      seq += 1;
      view.store.dispatch(
        applyChatEvent({
          ...event,
          chat_id: chatId,
          seq: String(seq),
        } as ChatEventEnvelope),
      );
    };
    const firstInfo = { ...record.latest_error, raw_error: "First 429" };
    const secondInfo = { ...record.latest_error, raw_error: "Second 503" };
    const runtimeRecord = (
      revision: number,
      patch: Partial<RecoveryRecord>,
    ): RecoveryRecord => ({
      ...record,
      revision,
      state: "waiting",
      retries_used: 0,
      latest_error: firstInfo,
      ...patch,
    });
    const stamp = (value: RecoveryRecord): RecoveryRecord => ({
      ...value,
      attempts: value.attempts.map((attempt) => ({ ...attempt, error: null })),
    });
    const attempt = (
      id: string,
      info: typeof firstInfo,
      extra: Record<string, unknown> = {},
    ): ChatMessage => ({
      role: "error",
      content: info.raw_error,
      message_id: id,
      extra: {
        _ui_only: true,
        error_info: info,
        recovery_operation_id: record.operation_id,
        recovery_run_id: record.run_id,
        ...extra,
      },
    });
    const anchorOf = (node: HTMLElement) =>
      node
        .closest("[data-testid='chat-virtuoso-item']")
        ?.getAttribute("data-chat-scroll-anchor-key");

    const first = runtimeRecord(1, {
      attempts: [
        { at_ms: 1, retry_number: 0, outcome: "failed", error: firstInfo },
      ],
    });
    apply({
      type: "message_added",
      index: 1,
      message: attempt("e1", firstInfo),
    });
    apply({
      type: "message_updated",
      message_id: "e1",
      message: attempt("e1", firstInfo, { recovery_record: stamp(first) }),
    });
    apply({ type: "recovery_updated", recovery: first });

    let panel = await screen.findByRole("region", { name: "Recovery status" });
    expect(anchorOf(panel)).toBe("e1");
    expect(panel).toHaveTextContent("First 429");
    expect(within(panel).queryByTestId("error-repeat-count")).toBeNull();
    expect(screen.queryByTestId("error-message-card")).toBeNull();

    const repeated = runtimeRecord(2, {
      retries_used: 1,
      attempts: [
        ...first.attempts,
        { at_ms: 2, retry_number: 1, outcome: "failed", error: firstInfo },
      ],
    });
    apply({
      type: "message_updated",
      message_id: "e1",
      message: attempt("e1", firstInfo, {
        repeat_count: 2,
        recovery_record: stamp(repeated),
      }),
    });
    apply({ type: "recovery_updated", recovery: repeated });
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "Recovery status" }),
      ).toHaveTextContent("×2"),
    );
    panel = screen.getByRole("region", { name: "Recovery status" });
    expect(within(panel).getByTestId("error-repeat-count")).toHaveTextContent(
      "×2",
    );
    expect(panel).toHaveTextContent("1 / 2 automatic retries used");

    const second = runtimeRecord(3, {
      retries_used: 2,
      latest_error: secondInfo,
      attempts: [
        ...repeated.attempts,
        { at_ms: 3, retry_number: 2, outcome: "failed", error: secondInfo },
      ],
    });
    apply({
      type: "message_added",
      index: 2,
      message: {
        role: "assistant",
        content: "Partial output",
        message_id: "p1",
        extra: { _ui_only: true, interrupted: true },
      },
    });
    apply({
      type: "message_added",
      index: 3,
      message: attempt("e2", secondInfo),
    });
    apply({
      type: "message_updated",
      message_id: "e2",
      message: attempt("e2", secondInfo, { recovery_record: stamp(second) }),
    });
    apply({ type: "recovery_updated", recovery: second });

    await waitFor(() =>
      expect(
        anchorOf(screen.getByRole("region", { name: "Recovery status" })),
      ).toBe("e2"),
    );
    panel = screen.getByRole("region", { name: "Recovery status" });
    expect(anchorOf(panel)).toBe("e2");
    expect(panel).toHaveTextContent("Second 503");
    expect(panel).not.toHaveTextContent("First 429");
    const earlier = screen.getByTestId("error-message-card");
    expect(anchorOf(earlier)).toBe("e1");
    expect(within(earlier).getByTestId("error-repeat-count")).toHaveTextContent(
      "×2",
    );
    expectEachShownOnce(["First 429", "Second 503"]);

    const succeeded = runtimeRecord(4, {
      state: "succeeded",
      retries_used: 3,
      latest_error: secondInfo,
      attempts: [
        ...second.attempts,
        { at_ms: 4, retry_number: 3, outcome: "succeeded" },
      ],
    });
    apply({
      type: "message_updated",
      message_id: "e2",
      message: attempt("e2", secondInfo, { recovery_record: stamp(succeeded) }),
    });
    apply({ type: "recovery_updated", recovery: succeeded });
    apply({
      type: "message_added",
      index: 4,
      message: { role: "assistant", content: "Recovered", message_id: "a1" },
    });

    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Recovery status" }),
      ).toBeNull(),
    );
    const cards = screen.getAllByTestId("error-message-card");
    expect(cards.map(anchorOf)).toEqual(["e1", "e2"]);
    expect(
      within(cards[1]).getByTestId("error-recovery-status"),
    ).toHaveTextContent("Recovered after 3 retries");
    expect(within(cards[0]).queryByTestId("error-recovery-status")).toBeNull();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    expectEachShownOnce(["First 429", "Second 503"]);
  });
});
