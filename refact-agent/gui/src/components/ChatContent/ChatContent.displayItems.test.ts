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
    const matchingErrors = (items ?? [])
      .filter((item) => item.type === "error")
      .flatMap((item) => (item.type === "error" ? item.errors : []))
      .filter((error) => error.content === failure.content);
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
