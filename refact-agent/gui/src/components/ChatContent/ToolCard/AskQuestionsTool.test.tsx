import { afterEach, describe, expect, test, vi } from "vitest";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
  within,
} from "../../../utils/test-utils";
import { AskQuestionsTool } from "./AskQuestionsTool";
import type { ChatMessage, ToolCall } from "../../../services/refact/types";

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
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("renders single_select as a readable vertical radio list", async () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.messages = [toolMessage()];

    const { user } = render(<AskQuestionsTool toolCall={toolCall()} />, {
      preloadedState: { chat },
    });

    const radiogroup = screen.getByRole("radiogroup", {
      name: "Choose the rollout path",
    });
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
});
