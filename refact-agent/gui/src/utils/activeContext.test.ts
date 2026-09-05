import { describe, expect, it } from "vitest";
import {
  computeActiveContext,
  describeActiveContextGate,
  isGenerationGatedByContext,
  activeContextPayloadMessageIds,
} from "./activeContext";
import type { ChatMessage, ChatMessages } from "../services/refact/types";

function user(content: string, id?: string): ChatMessage {
  return {
    role: "user",
    content,
    message_id: id ?? `user-${content}`,
  } as unknown as ChatMessage;
}

function assistant(content: string, id?: string): ChatMessage {
  return {
    role: "assistant",
    content,
    message_id: id ?? `assistant-${content}`,
  } as unknown as ChatMessage;
}

function reconstructedReport(
  payloadMessages: ChatMessage[],
  overrides: Record<string, unknown> = {},
  placement: "flat" | "extra" = "flat",
): ChatMessage {
  const metadata = {
    kind: "reconstructed_history",
    schema_version: 1,
    payload: { messages: payloadMessages },
    ...overrides,
  };
  return {
    role: "compression_report",
    content: "Context rebuilt",
    ...(placement === "flat"
      ? { compression_report: metadata }
      : { extra: { compression_report: metadata } }),
  } as unknown as ChatMessage;
}

function staticReport(): ChatMessage {
  return {
    role: "compression_report",
    content: "Compressed in place",
    compression_report: {
      kind: "chat_compression_report",
      context_files_removed: 3,
      tokens_before: 1000,
      tokens_after: 400,
    },
  } as unknown as ChatMessage;
}

function legacySummary(): ChatMessage {
  return {
    role: "summarization",
    content: "Older summary",
    summarization_tier: "tier1_llm",
    compression: { kind: "llm_segment_summary" },
  } as unknown as ChatMessage;
}

describe("computeActiveContext", () => {
  it("returns the transcript unchanged when there is no report", () => {
    const messages: ChatMessages = [user("hi"), assistant("hello")];
    const result = computeActiveContext(messages);

    expect(result.status).toBe("none");
    expect(result.active).toBe(messages);
    expect(result.transcript).toBe(messages);
    expect(result.anchorIndex).toBeNull();
    expect(isGenerationGatedByContext(result)).toBe(false);
  });

  it("expands the payload plus the suffix that follows the anchor", () => {
    const payload = [user("rebuilt user"), assistant("rebuilt assistant")];
    const messages: ChatMessages = [
      user("original 1"),
      assistant("original 2"),
      reconstructedReport(payload),
      user("after rebuild"),
    ];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("reconstructed");
    expect(result.active).toHaveLength(3);
    expect(result.active[0]).toBe(payload[0]);
    expect(result.active[1]).toBe(payload[1]);
    expect(result.active[2]).toBe(messages[3]);
    expect(result.suffixCount).toBe(1);
    expect(result.anchorIndex).toBe(2);
  });

  it("keeps the full transcript browseable even after a rebuild", () => {
    const messages: ChatMessages = [
      user("original"),
      reconstructedReport([user("rebuilt")]),
    ];

    const result = computeActiveContext(messages);

    expect(result.transcript).toBe(messages);
    expect(result.transcript).toHaveLength(2);
  });

  it("does not duplicate the pre-anchor messages into the active view", () => {
    const messages: ChatMessages = [
      user("original", "m1"),
      assistant("original reply", "m2"),
      reconstructedReport([user("rebuilt", "m1")]),
    ];

    const result = computeActiveContext(messages);

    expect(result.active).toHaveLength(1);
    expect(result.active[0].content).toBe("rebuilt");
  });

  it("uses the latest anchor when several rebuilds happened", () => {
    const messages: ChatMessages = [
      reconstructedReport([user("first rebuild")]),
      user("middle"),
      reconstructedReport([user("second rebuild")]),
      user("tail"),
    ];

    const result = computeActiveContext(messages);

    expect(result.anchorIndex).toBe(2);
    expect(result.active.map((m) => m.content)).toEqual([
      "second rebuild",
      "tail",
    ]);
  });

  it("reads metadata nested under extra as well as flattened", () => {
    const messages: ChatMessages = [
      reconstructedReport([user("from extra")], {}, "extra"),
    ];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("reconstructed");
    expect(result.active[0].content).toBe("from extra");
  });

  it("surfaces disclosure metadata from the anchor", () => {
    const messages: ChatMessages = [
      reconstructedReport([user("x")], {
        model: "gpt-test",
        trigger: "mode_transition",
        from_mode: "agent",
        to_mode: "plan",
        source_version: 7,
      }),
    ];

    const result = computeActiveContext(messages);

    expect(result.metadata?.model).toBe("gpt-test");
    expect(result.metadata?.trigger).toBe("mode_transition");
    expect(result.metadata?.from_mode).toBe("agent");
    expect(result.metadata?.to_mode).toBe("plan");
    expect(result.metadata?.source_version).toBe(7);
  });

  it("blocks instead of falling back when the schema version is unknown", () => {
    const messages: ChatMessages = [
      user("original"),
      reconstructedReport([user("x")], { schema_version: 99 }),
    ];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("blocked");
    expect(result.reason).toBe("unsupported_schema_version");
    expect(result.active).toEqual([]);
    expect(result.transcript).toBe(messages);
    expect(isGenerationGatedByContext(result)).toBe(true);
    expect(describeActiveContextGate(result)).toContain("newer format");
  });

  it("blocks when the payload is malformed", () => {
    const broken = {
      role: "compression_report",
      content: "bad",
      compression_report: {
        kind: "reconstructed_history",
        schema_version: 1,
        payload: { messages: "not-an-array" },
      },
    } as unknown as ChatMessage;

    const result = computeActiveContext([broken]);

    expect(result.status).toBe("blocked");
    expect(result.reason).toBe("malformed_payload");
    expect(result.active).toEqual([]);
  });

  it("blocks on a malformed newest anchor rather than using a stale older one", () => {
    const messages: ChatMessages = [
      reconstructedReport([user("stale but valid")]),
      reconstructedReport([user("x")], { schema_version: 42 }),
    ];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("blocked");
    expect(result.active).toEqual([]);
  });

  it("never treats a static compression report as an anchor", () => {
    const messages: ChatMessages = [user("a"), staticReport(), user("b")];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("none");
    expect(result.anchorIndex).toBeNull();
    expect(result.active).toBe(messages);
  });

  it("does not demand a rebuild for ordinary static compression", () => {
    const result = computeActiveContext([user("a"), staticReport()]);

    expect(result.status).toBe("none");
    expect(isGenerationGatedByContext(result)).toBe(false);
    expect(describeActiveContextGate(result)).toBeNull();
  });

  it("requires an explicit rebuild for legacy model segment summaries", () => {
    const messages: ChatMessages = [user("a"), legacySummary(), user("b")];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("legacy_rebuild_required");
    expect(isGenerationGatedByContext(result)).toBe(true);
    expect(describeActiveContextGate(result)).toContain("older format");
  });

  it("keeps the legacy transcript browseable but not usable as active plan", () => {
    const messages: ChatMessages = [user("a"), legacySummary()];

    const result = computeActiveContext(messages);

    expect(result.active).toEqual([]);
    expect(result.transcript).toBe(messages);
  });

  it("clears the legacy gate once a reconstructed report exists", () => {
    const messages: ChatMessages = [
      user("a"),
      legacySummary(),
      reconstructedReport([user("rebuilt")]),
    ];

    const result = computeActiveContext(messages);

    expect(result.status).toBe("reconstructed");
    expect(isGenerationGatedByContext(result)).toBe(false);
  });

  it("blocks legacy artifacts in a rebuilt suffix", () => {
    expect(
      computeActiveContext([
        reconstructedReport([user("rebuilt")]),
        legacySummary(),
      ]).status,
    ).toBe("legacy_rebuild_required");
  });

  it("recognizes source ids without tier or role assumptions", () => {
    const message = {
      ...user("legacy"),
      summarized_source_message_ids: [],
    } as unknown as ChatMessage;
    expect(computeActiveContext([message]).status).toBe(
      "legacy_rebuild_required",
    );
  });

  it("does not mistake a tier alone or static source ids for legacy", () => {
    expect(
      computeActiveContext([
        {
          ...user("ordinary"),
          summarization_tier: "tier1_llm",
        } as unknown as ChatMessage,
      ]).status,
    ).toBe("none");
    expect(
      computeActiveContext([
        {
          ...staticReport(),
          summarized_range: [0, 1],
          summarized_source_message_ids: ["old"],
        } as unknown as ChatMessage,
      ]).status,
    ).toBe("none");
  });

  it("blocks unknown report kinds, missing ids, duplicate ids and recursive payloads", () => {
    for (const report of [
      reconstructedReport([user("x")], { kind: "future_history" }),
      reconstructedReport([{ role: "user", content: "no id" }]),
      reconstructedReport([user("one", "same"), user("two", "same")]),
      reconstructedReport([reconstructedReport([user("nested")])]),
      reconstructedReport([]),
    ])
      expect(computeActiveContext([report]).status).toBe("blocked");
  });

  it("treats an empty thread as ungated", () => {
    const result = computeActiveContext([]);
    expect(result.status).toBe("none");
    expect(isGenerationGatedByContext(result)).toBe(false);
  });

  it("handles undefined messages", () => {
    const result = computeActiveContext(undefined);
    expect(result.status).toBe("none");
    expect(result.active).toEqual([]);
  });
});

describe("activeContextPayloadMessageIds", () => {
  it("collects ids from the rebuilt payload only", () => {
    const messages: ChatMessages = [
      user("original", "m1"),
      reconstructedReport([user("rebuilt", "p1"), assistant("r", "p2")]),
      user("tail", "m9"),
    ];

    const ids = activeContextPayloadMessageIds(computeActiveContext(messages));

    expect([...ids].sort()).toEqual(["p1", "p2"]);
  });

  it("is empty when there is no anchor", () => {
    const ids = activeContextPayloadMessageIds(
      computeActiveContext([user("a", "m1")]),
    );
    expect(ids.size).toBe(0);
  });
});
