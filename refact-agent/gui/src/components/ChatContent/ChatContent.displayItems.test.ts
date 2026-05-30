import { describe, expect, it } from "vitest";
import type { AssistantMessage, ChatMessages } from "../../services/refact";
import {
  buildDisplayItems,
  tryIncrementalDisplayItemsUpdate,
} from "./ChatContentDisplayItems";

function assistantMessage(
  overrides: Partial<AssistantMessage> = {},
): AssistantMessage {
  return {
    role: "assistant",
    content: "assistant content",
    message_id: "assistant-1",
    ...overrides,
  };
}

describe("ChatContent display items", () => {
  it("rebuilds a same-index assistant update into a summarization item when it becomes compressed", () => {
    const previousMessages: ChatMessages = [assistantMessage()];
    const nextMessages: ChatMessages = [
      assistantMessage({
        content: "compressed summary",
        extra: { compression: { kind: "llm_segment_summary" } },
      }),
    ];
    const previousItems = buildDisplayItems(previousMessages, false);

    const nextItems = tryIncrementalDisplayItemsUpdate(
      previousMessages,
      nextMessages,
      previousItems,
      false,
    );

    expect(nextItems).not.toBeNull();
    expect(nextItems).toHaveLength(1);
    expect(nextItems?.[0]?.type).toBe("summarization");
    expect(nextItems?.[0]?.messageIndex).toBe(0);
  });

  it("keeps ordinary same-index assistant updates on the incremental assistant path", () => {
    const previousMessages: ChatMessages = [assistantMessage()];
    const nextMessages: ChatMessages = [
      assistantMessage({ content: "streamed assistant content" }),
    ];
    const previousItems = buildDisplayItems(previousMessages, true);

    const nextItems = tryIncrementalDisplayItemsUpdate(
      previousMessages,
      nextMessages,
      previousItems,
      true,
    );

    expect(nextItems).not.toBeNull();
    expect(nextItems).toHaveLength(1);
    expect(nextItems?.[0]?.type).toBe("assistant");
    expect(nextItems?.[0]).not.toBe(previousItems[0]);
  });
});
