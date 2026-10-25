import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  render,
  screen,
  waitFor,
  createDefaultChatState,
} from "../../../utils/test-utils";
import { AskQuestionsTool } from "./AskQuestionsTool";
import type { PendingQuestionSet } from "../../../services/refact/types";

const call = {
  id: "ask-flex",
  index: 0,
  function: { name: "ask_questions", arguments: "{}" },
};
function setup(extra: Partial<PendingQuestionSet> = {}) {
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
  return render(<AskQuestionsTool toolCall={call} />, {
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
    const { user } = setup();
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
