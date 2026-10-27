import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  render,
  screen,
  waitFor,
  createDefaultChatState,
} from "../../../utils/test-utils";
import { AskQuestionsTool } from "./AskQuestionsTool";
import type { PendingQuestionSet } from "../../../services/refact/types";
import { act } from "react-dom/test-utils";
import { applyChatEvent } from "../../../features/Chat/Thread";
import { markQueuedCommandCancelled } from "../../../features/Chat/Thread/actions";
import { selectPendingQuestionSetsById } from "../../../features/Chat/Thread/selectors";
import { CollapsibleStoreProvider } from "../useStoredOpen";

const call = {
  id: "ask-flex",
  index: 0,
  function: { name: "ask_questions", arguments: "{}" },
};
function setup(extra: Partial<PendingQuestionSet> = {}, compact = false) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.id = "questions-test";
  chat.current_thread_id = "questions-test";
  chat.open_thread_ids = ["questions-test"];
  chat.threads = { "questions-test": runtime };
  chat.threads[chat.current_thread_id].thread.pending_questions = [
    {
      tool_call_id: call.id,
      asked_at_ms: 42,
      status: "pending",
      response_protocol_version: 2,
      questions: [
        {
          id: "one",
          type: "single_select",
          text: "Choose a direction",
          options: ["A", "B"],
        },
        {
          id: "many",
          type: "multi_select",
          text: "Choose features",
          options: ["X", "Y"],
        },
        { id: "yes", type: "yes_no", text: "Continue?" },
      ],
      ...extra,
    },
  ];
  return render(<AskQuestionsTool toolCall={call} compact={compact} />, {
    preloadedState: { chat },
  });
}

describe("flexible question responses", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  test("submits custom text, explicit none, skip, explanations and overall comments without coercion", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user, container } = setup();
    expect(container.querySelector("section")).toHaveAttribute(
      "data-open",
      "true",
    );
    expect(
      screen.queryByRole("button", { name: "Questions for you" }),
    ).toBeNull();
    await user.click(
      screen.getAllByRole("button", { name: "Write my own answer" })[0],
    );
    await user.type(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
      "Neither; combine them.",
    );
    await user.click(
      screen.getAllByRole("button", { name: "Add explanation" })[0],
    );
    await user.type(
      screen.getByRole("textbox", { name: "Explanation for question 1" }),
      "Keep it simple.",
    );
    await user.click(screen.getByRole("button", { name: "None of these" }));
    await user.click(screen.getAllByRole("button", { name: "Skip" })[2]);
    await user.type(
      screen.getByRole("textbox", { name: /Overall comments/ }),
      "Start small.",
    );
    expect(
      screen.getByText("2 answered · 1 skipped · 0 remaining"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const [, options] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(JSON.parse(String(options.body))).toMatchObject({
      type: "submit_question_response",
      tool_call_id: "ask-flex",
      asked_at_ms: 42,
      response: {
        answers: [
          {
            id: "one",
            disposition: "custom",
            answer: "Neither; combine them.",
            explanation: "Keep it simple.",
          },
          { id: "many", disposition: "answered", answer: [] },
          { id: "yes", disposition: "skipped" },
        ],
        overall_comments: "Start small.",
      },
    });
    expect(
      screen.getByRole("button", { name: "Submit response" }),
    ).toBeDisabled();
    expect(
      screen.getByText("Response sent, awaiting confirmation."),
    ).toBeInTheDocument();
  });

  test("skip is reversible, retains drafts, and skip remaining works without answers", async () => {
    const { user, unmount } = setup();
    await user.click(
      screen.getAllByRole("button", { name: "Write my own answer" })[0],
    );
    await user.type(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
      "My draft",
    );
    await user.click(screen.getAllByRole("button", { name: "Skip" })[0]);
    await user.click(screen.getByRole("button", { name: "Undo skip" }));
    expect(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
    ).toHaveValue("My draft");
    unmount();
    const restored = setup();
    expect(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
    ).toHaveValue("My draft");
    await restored.user.click(
      screen.getByRole("button", { name: "Skip remaining" }),
    );
    expect(
      screen.getByText("1 answered · 2 skipped · 0 remaining"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Submit response" }),
    ).toBeEnabled();
  });

  test("retry keeps the same request identity and disables Yes/No during send", async () => {
    const fetchMock = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user } = setup({
      questions: [{ id: "yes", type: "yes_no", text: "Continue?" }],
    });
    await user.click(screen.getByRole("radio", { name: "No" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const bodies = fetchMock.mock.calls.map(
      (c) =>
        JSON.parse(String((c[1] as RequestInit).body)) as {
          client_request_id: string;
        },
    );
    expect(bodies[0].client_request_id).toBe(bodies[1].client_request_id);
    expect(screen.getByRole("radio", { name: "No" })).toBeDisabled();
  });

  test("resolved responses render human-readable values, custom text, comments and skips", () => {
    setup({
      status: "answered",
      response: {
        answers: [
          { id: "one", disposition: "custom", answer: "Neither" },
          { id: "many", disposition: "skipped" },
          { id: "yes", disposition: "answered", answer: false },
        ],
        overall_comments: "Global context",
      },
    });
    expect(screen.getByText("Custom answer: Neither")).toBeInTheDocument();
    expect(screen.getByText("Skipped")).toBeInTheDocument();
    expect(screen.getByText("No")).toBeInTheDocument();
    expect(screen.getByText("Global context")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Submit response" }),
    ).not.toBeInTheDocument();
  });
});

function requestIds(fetchMock: ReturnType<typeof vi.fn>): string[] {
  return fetchMock.mock.calls.map(
    (c) =>
      (
        JSON.parse(String((c[1] as RequestInit).body)) as {
          client_request_id: string;
        }
      ).client_request_id,
  );
}

function updateQuestion(
  store: ReturnType<typeof setup>["store"],
  patch: Partial<PendingQuestionSet>,
) {
  const set = selectPendingQuestionSetsById(
    store.getState(),
    "questions-test",
  )[0];
  act(() => {
    store.dispatch(
      applyChatEvent({
        type: "snapshot",
        chat_id: "questions-test",
        seq: "1",
        thread: {
          id: "questions-test",
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
        pending_questions: [{ ...set, ...patch }],
      }),
    );
  });
}

describe("compact protocol-v2 question responses", () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  test("starts closed and keyboard expansion exposes the production controls without technical details", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user, container } = setup({}, true);
    const row = screen.getByRole("button", { expanded: false });
    expect(row).toHaveTextContent("Choose a direction (+2 questions)");
    expect(row).toHaveTextContent("Needs answer");
    expect(container.querySelectorAll('[data-density="compact"]')).toHaveLength(
      1,
    );
    expect(
      screen.queryByRole("button", { name: "Submit response" }),
    ).toBeNull();
    await user.tab();
    expect(row).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("button", { name: "More details" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Submit response" }),
    ).toBeDisabled();
    await user.tab();
    expect(screen.getAllByRole("button", { name: "Skip" })[0]).toHaveFocus();
    await user.click(
      screen.getAllByRole("button", { name: "Write my own answer" })[0],
    );
    await user.type(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
      "A different direction",
    );
    await user.click(
      screen.getAllByRole("button", { name: "Add explanation" })[0],
    );
    await user.type(
      screen.getByRole("textbox", { name: "Explanation for question 1" }),
      "Keep the draft",
    );
    await user.click(screen.getByRole("button", { name: "None of these" }));
    await user.click(screen.getAllByRole("button", { name: "Skip" })[2]);
    await user.type(
      screen.getByRole("textbox", { name: /Overall comments/ }),
      "Extra context",
    );
    expect(fetchMock).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const body = JSON.parse(
      String((fetchMock.mock.calls[0][1] as RequestInit).body),
    ) as unknown;
    expect(body).toMatchObject({
      type: "submit_question_response",
      asked_at_ms: 42,
      response: {
        answers: [
          {
            id: "one",
            disposition: "custom",
            answer: "A different direction",
            explanation: "Keep the draft",
          },
          { id: "many", disposition: "answered", answer: [] },
          { id: "yes", disposition: "skipped" },
        ],
        overall_comments: "Extra context",
      },
    });
    await user.click(row);
    expect(row).toHaveTextContent("Needs answer · Awaiting confirmation");
    expect(row).toHaveAttribute("aria-expanded", "false");
  });

  test.each(["answered", "cancelled"] as const)(
    "%s history keeps every response and option behind a closed secondary disclosure",
    async (status) => {
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      const { user } = setup(
        {
          status,
          response: {
            answers: [
              {
                id: "one",
                disposition: "custom",
                answer: "Neither",
                explanation: "A reason",
              },
              { id: "many", disposition: "skipped" },
              { id: "yes", disposition: "answered", answer: false },
            ],
            overall_comments: "Overall history",
          },
        },
        true,
      );
      const row = screen.getByRole("button", { expanded: false });
      expect(row).toHaveTextContent(
        status === "answered" ? "Answered" : "Withdrawn",
      );
      expect(row.closest("[data-status]")).toHaveAttribute(
        "data-status",
        status === "answered" ? "success" : "idle",
      );
      await user.click(row);
      const details = screen.getByRole("button", { name: "More details" });
      expect(details).toHaveAttribute("aria-expanded", "false");
      expect(
        screen.getAllByRole("term").filter((term) => !term.closest("[inert]")),
      ).toHaveLength(1);
      expect(screen.getByText("B").closest("[inert]")).not.toBeNull();
      await user.click(details);
      expect(screen.getByText("B")).toBeInTheDocument();
      expect(screen.getByText("A reason")).toBeInTheDocument();
      expect(screen.getByText("Overall history")).toBeInTheDocument();
      if (status === "answered") {
        expect(screen.getByText("Custom answer: Neither")).toBeInTheDocument();
        expect(screen.getByText("Skipped")).toBeInTheDocument();
        expect(screen.getByText("No")).toBeInTheDocument();
      } else {
        expect(screen.getAllByText("No response submitted")).toHaveLength(3);
      }
      expect(
        screen.queryByRole("button", { name: "Submit response" }),
      ).toBeNull();
      expect(fetchMock).not.toHaveBeenCalled();
    },
  );

  test("keeps closed choice through streaming and authoritative resolution without inferring an answer", async () => {
    const { store, user } = setup({}, true);
    const row = screen.getByRole("button", { expanded: false });
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "stream_started",
          chat_id: "questions-test",
          seq: "1",
          message_id: "stream-message",
        }),
      );
    });
    expect(row).toHaveAttribute("aria-expanded", "false");
    expect(row).toHaveTextContent("Needs answer");
    updateQuestion(store, {
      status: "answered",
      response: {
        answers: [
          { id: "one", disposition: "custom", answer: "Manual response" },
        ],
        overall_comments: "",
      },
    });
    expect(row).toHaveTextContent("Answered");
    expect(row).toHaveAttribute("aria-expanded", "false");
    await user.click(row);
    expect(
      screen.getByRole("button", { name: "More details" }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  test("honors thread tc storage on remount and preserves draft values across collapse", async () => {
    const view = setup({}, true);
    const values = new Map<string, boolean>([[`tc:${call.id}`, true]]);
    const storage = {
      get: (key: string) => values.get(key),
      set: (key: string, open: boolean) => values.set(key, open),
    };
    view.unmount();
    const mounted = render(
      <CollapsibleStoreProvider value={storage}>
        <AskQuestionsTool toolCall={call} compact />
      </CollapsibleStoreProvider>,
      { store: view.store },
    );
    const row = screen.getByRole("button", {
      name: /Choose a direction/,
      expanded: true,
    });
    await mounted.user.click(
      screen.getAllByRole("button", { name: "Write my own answer" })[0],
    );
    await mounted.user.type(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
      "A kept draft",
    );
    await mounted.user.click(row);
    mounted.unmount();
    const restored = render(
      <CollapsibleStoreProvider value={storage}>
        <AskQuestionsTool toolCall={call} compact />
      </CollapsibleStoreProvider>,
      { store: view.store },
    );
    expect(screen.getByRole("button", { expanded: false })).toHaveTextContent(
      "Needs answer",
    );
    await restored.user.click(screen.getByRole("button", { expanded: false }));
    expect(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
    ).toHaveValue("A kept draft");
    updateQuestion(view.store, {
      questions: [
        {
          id: "one",
          type: "single_select",
          text: "Choose an updated direction",
          options: ["A", "B"],
        },
      ],
    });
    const restoredRow = screen.getByRole("button", {
      name: /Choose an updated direction/,
      expanded: true,
    });
    expect(
      screen.getByRole("textbox", { name: "Your answer to question 1" }),
    ).toHaveValue("A kept draft");
    await restored.user.click(restoredRow);
    updateQuestion(view.store, {});
    expect(restoredRow).toHaveAttribute("aria-expanded", "false");
  });

  test("send failures and retry controls remain directly accessible and request identity stays stable", async () => {
    const fetchMock = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user } = setup(
      { questions: [{ id: "yes", type: "yes_no", text: "Continue?" }] },
      true,
    );
    const row = screen.getByRole("button", { expanded: false });
    await user.click(row);
    await user.click(screen.getByRole("radio", { name: "No" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("offline");
    await user.click(row);
    expect(row).toHaveTextContent("Send failed");
    expect(row.closest("[data-status]")).toHaveAttribute(
      "data-status",
      "error",
    );
    await user.click(row);
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("radio", { name: "No" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: "Yes" })).toBeDisabled();
    await user.click(
      screen.getByRole("button", { name: "Check status / retry" }),
    );
    expect(screen.getByRole("radio", { name: "No" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    const ids = fetchMock.mock.calls.map(
      (c) =>
        (
          JSON.parse(String((c[1] as RequestInit).body)) as {
            client_request_id: string;
          }
        ).client_request_id,
    );
    expect(new Set(ids).size).toBe(1);
  });

  test("stale acknowledgment cannot reject a replacement question instance", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user, store } = setup({}, true);
    await user.click(screen.getByRole("button", { expanded: false }));
    await user.click(screen.getByRole("button", { name: "Skip remaining" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    const old = JSON.parse(
      String((fetchMock.mock.calls[0][1] as RequestInit).body),
    ) as { client_request_id: string };
    updateQuestion(store, { asked_at_ms: 43 });
    act(() => {
      store.dispatch(
        applyChatEvent({
          type: "ack",
          chat_id: "questions-test",
          seq: "2",
          client_request_id: old.client_request_id,
          accepted: false,
          result: null,
        }),
      );
    });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(
      screen.getByText("0 answered · 0 skipped · 3 remaining"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Skip remaining" }),
    ).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Skip remaining" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const next = JSON.parse(
      String((fetchMock.mock.calls[1][1] as RequestInit).body),
    ) as { client_request_id: string; asked_at_ms: number };
    expect(next.client_request_id).not.toBe(old.client_request_id);
    expect(next.asked_at_ms).toBe(43);
  });

  test("confirmed queue cancellation unlocks the kept draft and resubmits with a new identity", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user, store } = setup({
      questions: [{ id: "yes", type: "yes_no", text: "Continue?" }],
    });
    await user.click(screen.getByRole("radio", { name: "No" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("radio", { name: "No" })).toBeDisabled();
    const first = requestIds(fetchMock)[0];
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({
          chatId: "other-thread",
          clientRequestId: first,
        }),
      );
      store.dispatch(
        markQueuedCommandCancelled({
          chatId: "questions-test",
          clientRequestId: "unrelated",
        }),
      );
    });
    expect(screen.getByRole("radio", { name: "No" })).toBeDisabled();
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({
          chatId: "questions-test",
          clientRequestId: first,
        }),
      );
    });
    expect(screen.getByRole("radio", { name: "No" })).toBeEnabled();
    expect(screen.getByRole("radio", { name: "No" })).toBeChecked();
    expect(
      screen.queryByText("Response sent, awaiting confirmation."),
    ).toBeNull();
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(requestIds(fetchMock)[1]).not.toBe(first);
  });

  test("a response POST settling after confirmed cancellation does not relock", async () => {
    let resolvePost: (value: { ok: boolean }) => void = () => undefined;
    const fetchMock = vi.fn().mockReturnValueOnce(
      new Promise((resolve) => {
        resolvePost = resolve;
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { user, store } = setup({
      questions: [{ id: "yes", type: "yes_no", text: "Continue?" }],
    });
    await user.click(screen.getByRole("radio", { name: "No" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({
          chatId: "questions-test",
          clientRequestId: requestIds(fetchMock)[0],
        }),
      );
    });
    await act(async () => {
      resolvePost({ ok: true });
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: "No" })).toBeEnabled(),
    );
    expect(
      screen.queryByText("Response sent, awaiting confirmation."),
    ).toBeNull();
  });

  test("stale cancellation of an old request cannot unlock a replacement instance", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    const { user, store } = setup({
      questions: [{ id: "yes", type: "yes_no", text: "Continue?" }],
    });
    await user.click(screen.getByRole("radio", { name: "No" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    updateQuestion(store, { asked_at_ms: 43 });
    await user.click(screen.getByRole("radio", { name: "Yes" }));
    await user.click(screen.getByRole("button", { name: "Submit response" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const [old, next] = requestIds(fetchMock);
    expect(next).not.toBe(old);
    act(() => {
      store.dispatch(
        markQueuedCommandCancelled({
          chatId: "questions-test",
          clientRequestId: old,
        }),
      );
    });
    expect(screen.getByRole("radio", { name: "Yes" })).toBeDisabled();
    expect(
      screen.getByText("Response sent, awaiting confirmation."),
    ).toBeInTheDocument();
  });

  test("unrecognized historical state is neutral, escaped and never offers live controls", async () => {
    const { user, container } = setup(
      {
        status: "expired" as PendingQuestionSet["status"],
        questions: [
          { id: "one", type: "free_text", text: "<script>Question</script>" },
        ],
      },
      true,
    );
    const row = screen.getByRole("button", { expanded: false });
    expect(row).toHaveTextContent("State unknown");
    expect(row.closest("[data-status]")).toHaveAttribute("data-status", "idle");
    expect(container.querySelector("script")).toBeNull();
    await user.click(row);
    await user.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.queryByRole("button", { name: "Submit response" }),
    ).toBeNull();
  });
});
