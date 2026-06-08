import { describe, expect, test } from "vitest";
import {
  createDefaultChatState,
  render,
  screen,
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
});
