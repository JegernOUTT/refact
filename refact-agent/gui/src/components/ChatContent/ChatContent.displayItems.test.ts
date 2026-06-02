import { describe, expect, it } from "vitest";
import type {
  AssistantMessage,
  CDInstructionMessage,
  ChatMessages,
  CompressionReportMessage,
  EventMessage,
} from "../../services/refact";
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

function activateSkillOnlyAssistantMessage(
  overrides: Partial<AssistantMessage> = {},
): AssistantMessage {
  return assistantMessage({
    content: "",
    tool_calls: [
      {
        id: "activate-skill-1",
        index: 0,
        type: "function",
        function: {
          name: "activate_skill",
          arguments: JSON.stringify({ skill_name: "frog-skill" }),
        },
      },
    ],
    ...overrides,
  });
}

function expectIncrementalSameIndexMatchesFull(
  previousMessages: ChatMessages,
  nextMessages: ChatMessages,
): void {
  const previousItems = buildDisplayItems(previousMessages, false);

  const incrementalItems = tryIncrementalDisplayItemsUpdate(
    previousMessages,
    nextMessages,
    previousItems,
    false,
  );

  expect(incrementalItems).not.toBeNull();
  expect(incrementalItems).toEqual(buildDisplayItems(nextMessages, false));
}

function compressionReportMessage(
  overrides: Partial<CompressionReportMessage> = {},
): CompressionReportMessage {
  return {
    role: "compression_report",
    content: "## Chat compression report\n\n- Context files removed: 1",
    message_id: "compression-report-1",
    summarization_tier: "tier2_reactive",
    summarized_token_estimate: 42,
    extra: {
      compression_report: {
        kind: "chat_compression_report",
      },
    },
    ...overrides,
  };
}

function eventMessage(overrides: Partial<EventMessage> = {}): EventMessage {
  const subkind = overrides.subkind ?? "system_notice";
  const source = overrides.source ?? "chat.summarizer";
  return {
    role: "event",
    content: "Context compression failed: provider timeout",
    message_id: "event-1",
    subkind,
    source,
    extra: {
      event: {
        subkind,
        source,
        payload: {},
      },
    },
    ...overrides,
  };
}

function cdInstructionMessage(content: string): CDInstructionMessage {
  return {
    role: "cd_instruction",
    content,
  };
}

function expectIncrementalAppendMatchesFull(
  appendedMessage: ChatMessages[number],
): void {
  const previousMessages: ChatMessages = [
    assistantMessage({ message_id: "assistant-before" }),
  ];
  const nextMessages: ChatMessages = [...previousMessages, appendedMessage];
  const previousItems = buildDisplayItems(previousMessages, false);

  const incrementalItems = tryIncrementalDisplayItemsUpdate(
    previousMessages,
    nextMessages,
    previousItems,
    false,
  );

  expect(incrementalItems).not.toBeNull();
  expect(incrementalItems).toEqual(buildDisplayItems(nextMessages, false));
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

  it("matches full rebuild when an assistant message becomes a compressed summary", () => {
    const previousMessages: ChatMessages = [assistantMessage()];
    const nextMessages: ChatMessages = [
      assistantMessage({
        content: "compressed summary",
        extra: { compression: { kind: "llm_segment_summary" } },
      }),
    ];
    const previousItems = buildDisplayItems(previousMessages, false);

    const incrementalItems = tryIncrementalDisplayItemsUpdate(
      previousMessages,
      nextMessages,
      previousItems,
      false,
    );

    expect(incrementalItems).not.toBeNull();
    expect(incrementalItems).toEqual(buildDisplayItems(nextMessages, false));
  });

  it("renders assistant messages with top-level compression as summarization display items", () => {
    const messages: ChatMessages = [
      assistantMessage({
        content: "persisted compressed summary",
        compression: {
          kind: "llm_segment_summary",
          source_message_ids: ["user-1", "assistant-1"],
          summary_model: "summary-model",
        },
      }),
    ];

    const items = buildDisplayItems(messages, false);

    expect(items).toHaveLength(1);
    expect(items[0]?.type).toBe("summarization");
    if (items[0]?.type !== "summarization") {
      throw new Error("Expected summarization item");
    }
    expect(items[0].message.extra).toEqual({
      compression: {
        kind: "llm_segment_summary",
        source_message_ids: ["user-1", "assistant-1"],
        summary_model: "summary-model",
      },
    });
  });

  it("matches full rebuild when same-index activate_skill-only assistant becomes visible", () => {
    const previousMessages: ChatMessages = [
      activateSkillOnlyAssistantMessage(),
    ];
    const nextMessages: ChatMessages = [
      activateSkillOnlyAssistantMessage({ content: "Skill is activated now." }),
    ];

    expectIncrementalSameIndexMatchesFull(previousMessages, nextMessages);
  });

  it("matches full rebuild when same-index visible assistant becomes activate_skill-only hidden", () => {
    const previousMessages: ChatMessages = [
      activateSkillOnlyAssistantMessage({ content: "Skill is activated now." }),
    ];
    const nextMessages: ChatMessages = [activateSkillOnlyAssistantMessage()];

    expectIncrementalSameIndexMatchesFull(previousMessages, nextMessages);
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

  it("renders compression_report messages as summarization display items", () => {
    const messages: ChatMessages = [
      assistantMessage({ message_id: "assistant-before" }),
      compressionReportMessage(),
      assistantMessage({ message_id: "assistant-after" }),
    ];

    const items = buildDisplayItems(messages, false);

    expect(items).toHaveLength(3);
    expect(items[1]?.type).toBe("summarization");
    if (items[1]?.type !== "summarization") {
      throw new Error("Expected summarization item");
    }
    expect(items[1].messageIndex).toBe(1);
    expect(items[1].message.summarization_tier).toBe("tier2_reactive");
    expect(items[1].message.summarized_token_estimate).toBe(42);
    expect(items[1].message.extra).toEqual({
      compression_report: { kind: "chat_compression_report" },
    });
  });

  it("renders compression_report messages with top-level metadata as summarization display items", () => {
    const messages: ChatMessages = [
      compressionReportMessage({
        extra: undefined,
        compression_report: {
          kind: "chat_compression_report",
          context_files_removed: 2,
          estimated_tokens_saved: 3000,
        },
      }),
    ];

    const items = buildDisplayItems(messages, false);

    expect(items).toHaveLength(1);
    expect(items[0]?.type).toBe("summarization");
    if (items[0]?.type !== "summarization") {
      throw new Error("Expected summarization item");
    }
    expect(items[0].message.extra).toEqual({
      compression_report: {
        kind: "chat_compression_report",
        context_files_removed: 2,
        estimated_tokens_saved: 3000,
      },
    });
  });

  it("matches full rebuild when appending a compression_report message", () => {
    expectIncrementalAppendMatchesFull(compressionReportMessage());
  });

  it("renders chat summarizer compression failure events as error display items", () => {
    const failure = eventMessage();
    const messages: ChatMessages = [
      assistantMessage({ message_id: "assistant-before" }),
      failure,
      assistantMessage({ message_id: "assistant-after" }),
    ];

    const items = buildDisplayItems(messages, false);

    expect(items).toHaveLength(3);
    expect(items[1]?.type).toBe("error");
    if (items[1]?.type !== "error") {
      throw new Error("Expected error item");
    }
    expect(items[1].messageIndex).toBe(1);
    expect(items[1].errors).toHaveLength(1);
    expect(items[1].errors[0]?.content).toBe(failure.content);
  });

  it("matches full rebuild when appending a visible compression failure event", () => {
    expectIncrementalAppendMatchesFull(eventMessage());
  });

  it("keeps unrelated events and plan deltas hidden", () => {
    const messages: ChatMessages = [
      eventMessage({
        message_id: "unrelated-system-notice",
        content: "System notice unrelated to compression",
      }),
      eventMessage({
        message_id: "other-source-failure",
        source: "scheduler.cron",
      }),
      eventMessage({
        message_id: "plan-delta",
        subkind: "plan_delta",
        source: "tool.update_plan",
        content: "Context compression failed: not a summarizer notice",
      }),
    ];

    const items = buildDisplayItems(messages, false);

    expect(items).toHaveLength(0);
  });

  it("renders compression failure content once when appended incrementally", () => {
    const failure = eventMessage();
    const previousMessages: ChatMessages = [assistantMessage()];
    const nextMessages: ChatMessages = [...previousMessages, failure];
    const previousItems = buildDisplayItems(previousMessages, false);

    const items = tryIncrementalDisplayItemsUpdate(
      previousMessages,
      nextMessages,
      previousItems,
      false,
    );

    expect(items).not.toBeNull();
    const matchingErrors = (items ?? []).flatMap((item) =>
      item.type === "error"
        ? item.errors.filter((error) => error.content === failure.content)
        : [],
    );
    expect(matchingErrors).toHaveLength(1);
  });

  it("still renders skill activation cd_instruction messages", () => {
    const header = JSON.stringify({
      name: "frog-skill",
      allowed_tools: ["cat"],
      model_override: null,
    });
    const messages: ChatMessages = [
      cdInstructionMessage(`💿 SKILL_ACTIVATED ${header}\nSkill body`),
    ];

    const items = buildDisplayItems(messages, false);

    expect(items).toHaveLength(1);
    expect(items[0]?.type).toBe("skill_activated");
  });
});
