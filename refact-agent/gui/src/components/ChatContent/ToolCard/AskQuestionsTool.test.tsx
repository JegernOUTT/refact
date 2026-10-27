import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
  within,
} from "../../../utils/test-utils";
import { AskQuestionsTool } from "./AskQuestionsTool";
import { setUpStore } from "../../../app/store";
import {
  backUpMessages,
  clearSseRefreshRequest,
} from "../../../features/Chat/Thread/actions";
import type {
  ChatMessage,
  PendingQuestionSet,
  ToolCall,
} from "../../../services/refact/types";
import { clearAskQuestionsDraft } from "../../../utils/chatUiPersistence";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import { applyChatEvent } from "../../../features/Chat/Thread";
import { markQueuedCommandCancelled } from "../../../features/Chat/Thread/actions";
import { act } from "react-dom/test-utils";

function toolCall(): ToolCall {
  return {
    id: "ask-questions-single-select",
    index: 0,
    function: {
      name: "ask_questions",
      arguments: "{}",
    },
  };
}

function toolMessage(): ChatMessage {
  return {
    role: "tool",
    tool_call_id: "ask-questions-single-select",
    content: JSON.stringify({
      type: "ask_questions",
      tool_call_id: "ask-questions-single-select",
      questions: [
        {
          id: "priority_path",
          type: "single_select",
          text: "Choose the rollout path",
          options: [
            "— 3 cards P1 → P2 → P3 with isolated worktrees and verification after each card",
            "One larger card that keeps all settings changes together for review",
            "Pause this stream and only collect questions before implementation",
          ],
        },
      ],
    }),
  };
}

describe("AskQuestionsTool", () => {
  beforeEach(() => clearAskQuestionsDraft(toolCall().id));

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("renders single_select as a readable vertical radio list", async () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.messages = [toolMessage()];

    const { user, container } = render(
      <AskQuestionsTool toolCall={toolCall()} />,
      {
        preloadedState: { chat },
      },
    );

    const radiogroup = screen.getByRole("radiogroup", {
      name: "Choose the rollout path",
    });
    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(
      screen.queryByRole("button", { name: "Questions for you" }),
    ).toBeNull();
    const radios = within(radiogroup).getAllByRole("radio");

    expect(radios).toHaveLength(3);
    expect(
      within(radiogroup).getByText(
        "— 3 cards P1 → P2 → P3 with isolated worktrees and verification after each card",
      ),
    ).toBeInTheDocument();
    expect(radios[0]).not.toBeChecked();

    await user.click(radios[0]);

    expect(radios[0]).toBeChecked();
    expect(radios[1]).not.toBeChecked();
  });

  test("submits native pending answers and waits for the authoritative snapshot", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.pending_questions = [
      {
        tool_call_id: "ask-questions-single-select",
        asked_at_ms: 1,
        status: "pending",
        questions: [
          { id: "continue", type: "yes_no", text: "Continue?" },
          {
            id: "scope",
            type: "single_select",
            text: "Choose scope",
            options: ["GUI", "Engine"],
          },
        ],
      },
    ];

    const { user } = render(<AskQuestionsTool toolCall={toolCall()} />, {
      preloadedState: { chat },
    });

    expect(
      screen.getByRole("button", { name: "Submit Answers" }),
    ).toBeDisabled();
    await user.click(screen.getByText("Yes"));
    await user.click(screen.getByRole("radio", { name: "GUI" }));
    await user.click(screen.getByRole("button", { name: "Submit Answers" }));

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, request] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(request.body))).toMatchObject({
      type: "answer_questions",
      tool_call_id: "ask-questions-single-select",
      answers: [
        { id: "continue", answer: true },
        { id: "scope", answer: "GUI" },
      ],
    });
    expect(
      screen.getByText("Answers sent, awaiting confirmation."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Submit Answers" }),
    ).toBeDisabled();
  });

  test("legacy native answers unlock with kept draft after confirmed queue cancellation only", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    const chatId = runtime.thread.id;
    runtime.thread.pending_questions = [
      {
        tool_call_id: "ask-questions-single-select",
        asked_at_ms: 1,
        status: "pending",
        questions: [
          {
            id: "scope",
            type: "single_select",
            text: "Choose scope",
            options: ["GUI", "Engine"],
          },
        ],
      },
    ];
    const { user, store } = render(<AskQuestionsTool toolCall={toolCall()} />, {
      preloadedState: { chat },
    });
    await user.click(screen.getByRole("radio", { name: "GUI" }));
    await user.click(screen.getByRole("button", { name: "Submit Answers" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const first = (
      JSON.parse(
        String((fetchMock.mock.calls[0] as [string, RequestInit])[1].body),
      ) as { client_request_id: string }
    ).client_request_id;
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({ chatId, clientRequestId: "unrelated" }),
      );
    });
    expect(screen.getByRole("radio", { name: "GUI" })).toBeDisabled();
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({ chatId, clientRequestId: first }),
      );
    });
    expect(screen.getByRole("radio", { name: "GUI" })).toBeEnabled();
    expect(screen.getByRole("radio", { name: "GUI" })).toBeChecked();
    expect(
      screen.queryByText("Answers sent, awaiting confirmation."),
    ).toBeNull();
    await user.click(screen.getByRole("radio", { name: "Engine" }));
    await user.click(screen.getByRole("button", { name: "Submit Answers" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const second = JSON.parse(
      String((fetchMock.mock.calls[1] as [string, RequestInit])[1].body),
    ) as { client_request_id: string; answers: unknown };
    expect(second.client_request_id).not.toBe(first);
    expect(second.answers).toEqual([{ id: "scope", answer: "Engine" }]);
  });

  test("a delayed legacy submission and its cancellation cannot affect a replacement question", async () => {
    let resolveOld!: (response: { ok: boolean }) => void;
    const fetchMock = vi
      .fn()
      .mockReturnValueOnce(
        new Promise<{ ok: boolean }>((resolve) => {
          resolveOld = resolve;
        }),
      )
      .mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    const runtime = chat.threads[chatId];
    const questionSet: PendingQuestionSet = {
      tool_call_id: toolCall().id ?? "",
      asked_at_ms: 1,
      status: "pending",
      questions: [
        {
          id: "scope",
          type: "single_select",
          text: "Choose scope",
          options: ["GUI", "Engine"],
        },
      ],
    };
    runtime.thread.pending_questions = [questionSet];
    const { user, store } = render(<AskQuestionsTool toolCall={toolCall()} />, {
      preloadedState: { chat },
    });
    await user.click(screen.getByRole("radio", { name: "GUI" }));
    await user.click(screen.getByRole("button", { name: "Submit Answers" }));
    const old = JSON.parse(
      String((fetchMock.mock.calls[0] as [string, RequestInit])[1].body),
    ) as { client_request_id: string };
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "snapshot",
          chat_id: chatId,
          seq: "1",
          thread: {
            id: chatId,
            title: "Questions",
            model: "",
            mode: "agent",
            tool_use: "agent",
            boost_reasoning: false,
            context_tokens_cap: null,
            include_project_info: true,
            checkpoints_enabled: true,
            is_title_generated: false,
          },
          runtime: {
            state: "idle",
            paused: false,
            error: null,
            queue_size: 0,
            pause_reasons: [],
            queued_items: [],
          },
          messages: [],
          background_agents: [],
          pending_questions: [{ ...questionSet, asked_at_ms: 2 }],
        }),
      );
    });
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: "Engine" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("radio", { name: "Engine" }));
    await user.click(screen.getByRole("button", { name: "Submit Answers" }));
    await screen.findByText("Answers sent, awaiting confirmation.");
    await act(async () => {
      resolveOld({ ok: true });
      await Promise.resolve();
    });
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({
          chatId,
          clientRequestId: old.client_request_id,
        }),
      );
    });
    expect(screen.getByRole("radio", { name: "Engine" })).toBeDisabled();
    expect(
      screen.getByText("Answers sent, awaiting confirmation."),
    ).toBeVisible();
  });

  test("renders answered and cancelled native questions without a submit button", () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.pending_questions = [
      {
        tool_call_id: "ask-questions-single-select",
        asked_at_ms: 1,
        status: "answered",
        questions: [{ id: "continue", type: "yes_no", text: "Continue?" }],
        answers: [{ id: "continue", answer: true }],
      },
    ];

    const { rerender } = render(<AskQuestionsTool toolCall={toolCall()} />, {
      preloadedState: { chat },
    });
    expect(screen.queryByRole("button", { name: "Submit Answers" })).toBeNull();
    expect(screen.getByText("Questions answered")).toBeInTheDocument();

    runtime.thread.pending_questions[0].status = "cancelled";
    rerender(<AskQuestionsTool toolCall={toolCall()} />);
    expect(screen.queryByRole("button", { name: "Submit Answers" })).toBeNull();
  });

  test("compact pending questions require one keyboard expansion and an explicit answer submit", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.pending_questions = [
      {
        tool_call_id: toolCall().id ?? "",
        asked_at_ms: 1,
        status: "pending",
        questions: [
          {
            id: "scope",
            type: "single_select",
            text: "Choose scope",
            options: ["GUI", "Engine"],
          },
        ],
      },
    ];
    const { user, container, store } = render(
      <AskQuestionsTool toolCall={toolCall()} compact />,
      { preloadedState: { chat } },
    );
    const row = screen.getByRole("button", { expanded: false });
    expect(row).toHaveTextContent("Choose scope");
    expect(row).toHaveTextContent("Needs answer");
    expect(container.querySelector('[data-density="compact"]')).not.toBeNull();
    expect(screen.queryByRole("button", { name: "Submit Answers" })).toBeNull();
    await user.tab();
    expect(row).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(row).toHaveAttribute("aria-expanded", "true");
    expect(screen.queryByRole("button", { name: "More details" })).toBeNull();
    let submit = screen.getByRole("button", { name: "Submit Answers" });
    expect(submit).toBeDisabled();
    await user.tab();
    expect(screen.getByRole("radio", { name: "GUI" })).toHaveFocus();
    await user.keyboard(" ");
    expect(screen.getByRole("radio", { name: "GUI" })).toBeChecked();
    expect(fetchMock).not.toHaveBeenCalled();
    await user.click(row);
    expect(row).toHaveAttribute("aria-expanded", "false");
    await user.click(row);
    expect(screen.getByRole("radio", { name: "GUI" })).toBeChecked();
    submit = screen.getByRole("button", { name: "Submit Answers" });
    screen.getByRole("radio", { name: "GUI" }).focus();
    await user.tab();
    expect(submit).toHaveFocus();
    await user.keyboard("{Enter}");
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(row).toHaveTextContent("Needs answer");
    expect(
      screen.getByText("Answers sent, awaiting confirmation."),
    ).toBeInTheDocument();
    expect(submit).toBeDisabled();
    await user.click(row);
    store.dispatch(
      applyChatEvent({
        type: "snapshot",
        chat_id: chat.current_thread_id,
        seq: "1",
        thread: {
          id: chat.current_thread_id,
          title: "Questions",
          model: "",
          mode: "agent",
          tool_use: "agent",
          boost_reasoning: false,
          context_tokens_cap: null,
          include_project_info: true,
          checkpoints_enabled: true,
          is_title_generated: false,
        },
        runtime: {
          state: "idle",
          paused: false,
          error: null,
          queue_size: 0,
          pause_reasons: [],
          queued_items: [],
        },
        messages: [],
        background_agents: [],
        pending_questions: [
          {
            ...runtime.thread.pending_questions[0],
            status: "answered",
            answers: [{ id: "scope", answer: "GUI" }],
          },
        ],
      }),
    );
    await waitFor(() => expect(row).toHaveTextContent("Answered"));
    expect(row).toHaveAttribute("aria-expanded", "false");
  });

  test.each(["answered", "cancelled"] as const)(
    "compact %s history preserves options and answers behind explicit details",
    async (status) => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      const chat = createDefaultChatState();
      chat.threads[chat.current_thread_id].thread.pending_questions = [
        {
          tool_call_id: toolCall().id ?? "",
          asked_at_ms: 1,
          status,
          questions: [
            {
              id: "scope",
              type: "single_select",
              text: "Choose scope",
              options: ["GUI", "Engine"],
            },
          ],
          answers:
            status === "answered"
              ? [{ id: "scope", answer: "GUI" }]
              : undefined,
        },
      ];
      const { user } = render(
        <AskQuestionsTool toolCall={toolCall()} compact />,
        { preloadedState: { chat } },
      );
      const row = screen.getByRole("button", { expanded: false });
      expect(row).toHaveTextContent(
        status === "answered" ? "Answered" : "Withdrawn",
      );
      await user.click(row);
      expect(
        screen.getByRole("button", { name: "More details" }),
      ).toHaveAttribute("aria-expanded", "false");
      expect(screen.getByText("Engine").closest("[inert]")).not.toBeNull();
      await user.click(screen.getByRole("button", { name: "More details" }));
      expect(screen.getByText("Engine")).toBeInTheDocument();
      expect(
        screen.getByText(status === "answered" ? /→ GUI/ : /no answer/),
      ).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: "Submit Answers" }),
      ).toBeNull();
      expect(fetchMock).not.toHaveBeenCalled();
    },
  );

  test("compact legacy question retains direct answer controls and does not infer an answer from unrelated text", async () => {
    const chat = createDefaultChatState();
    const threadId = chat.current_thread_id;
    chat.threads[threadId].thread.messages = [toolMessage()];
    const store = setUpStore({ chat });
    const { user } = render(
      <AskQuestionsTool toolCall={toolCall()} compact />,
      { store },
    );
    const row = screen.getByRole("button", { expanded: false });
    expect(row).toHaveTextContent("Needs answer");
    await user.click(row);
    expect(
      screen.getByRole("radiogroup", { name: "Choose the rollout path" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Submit Answers" }),
    ).toBeInTheDocument();
    const radios = screen.getAllByRole("radio");
    await user.click(radios[0]);
    await user.type(
      screen.getByPlaceholderText("Add any extra context..."),
      "Keep my context",
    );
    await user.click(row);
    await user.click(row);
    expect(screen.getAllByRole("radio")[0]).toBeChecked();
    expect(screen.getByPlaceholderText("Add any extra context...")).toHaveValue(
      "Keep my context",
    );
    await user.click(row);
    store.dispatch(
      backUpMessages({
        id: threadId,
        messages: [
          toolMessage(),
          { role: "user", content: "Move on to something else" },
        ],
      }),
    );
    await waitFor(() => expect(row).toHaveTextContent("State unknown"));
    expect(row).toHaveAttribute("aria-expanded", "false");
  });

  test("compact history honors stored open state and does not manufacture expired state", () => {
    const chat = createDefaultChatState();
    chat.threads[chat.current_thread_id].thread.pending_questions = [
      {
        tool_call_id: toolCall().id ?? "",
        asked_at_ms: 1,
        status: "expired" as PendingQuestionSet["status"],
        questions: [
          {
            id: "scope",
            type: "free_text",
            text: "<script>Choose scope</script>",
          },
        ],
      },
    ];
    const { container } = render(
      <CollapsibleStoreProvider value={{ get: () => true, set: vi.fn() }}>
        <AskQuestionsTool toolCall={toolCall()} compact />
      </CollapsibleStoreProvider>,
      { preloadedState: { chat } },
    );
    const row = screen.getByRole("button", { expanded: true });
    expect(row).toHaveTextContent("State unknown");
    expect(row).not.toHaveTextContent("Expired");
    expect(
      screen.getByRole("button", { name: "More details" }),
    ).toHaveAttribute("aria-expanded", "false");
    expect(container.querySelector("script")).toBeNull();
  });

  test("compact syncing history keeps the complete questions recoverable without offering stale controls", async () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.messages = [toolMessage()];
    runtime.thread.pending_questions = [];
    const { user } = render(
      <AskQuestionsTool toolCall={toolCall()} compact />,
      { preloadedState: { chat } },
    );
    const row = screen.getByRole("button", { expanded: false });
    expect(row).toHaveTextContent("Syncing state");
    await user.click(row);
    await user.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.getByText(
        "One larger card that keeps all settings changes together for review",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Submit Answers" })).toBeNull();
  });

  describe("pending question sync self-heal", () => {
    test("requests one SSE refresh while the question set is still syncing", () => {
      const chat = createDefaultChatState();
      const threadId = chat.current_thread_id;
      const runtime = chat.threads[threadId];
      runtime.thread.messages = [toolMessage()];
      runtime.thread.pending_questions = [];

      const store = setUpStore({ chat });
      const { rerender } = render(<AskQuestionsTool toolCall={toolCall()} />, {
        store,
      });

      expect(screen.getByText("Questions syncing")).toBeInTheDocument();
      expect(store.getState().chat.sse_refresh_requested).toBe(threadId);

      store.dispatch(clearSseRefreshRequest());
      store.dispatch(
        backUpMessages({ id: threadId, messages: [toolMessage()] }),
      );
      rerender(<AskQuestionsTool toolCall={toolCall()} />);

      expect(store.getState().chat.sse_refresh_requested).toBeNull();
    });

    test("does not refresh when the engine never advertised pending questions", () => {
      const chat = createDefaultChatState();
      const runtime = chat.threads[chat.current_thread_id];
      runtime.thread.messages = [toolMessage()];

      const store = setUpStore({ chat });
      render(<AskQuestionsTool toolCall={toolCall()} />, { store });

      expect(store.getState().chat.sse_refresh_requested).toBeNull();
    });

    test("does not refresh for a historical question the chat already moved past", () => {
      const chat = createDefaultChatState();
      const runtime = chat.threads[chat.current_thread_id];
      runtime.thread.messages = [
        toolMessage(),
        {
          role: "user",
          content: JSON.stringify({
            type: "ask_questions_answer",
            tool_call_id: "ask-questions-single-select",
            answers: [{ id: "priority_path", answer: "already answered" }],
          }),
        },
      ];
      runtime.thread.pending_questions = [];

      const store = setUpStore({ chat });
      render(<AskQuestionsTool toolCall={toolCall()} />, { store });

      expect(store.getState().chat.sse_refresh_requested).toBeNull();
    });
  });
});
