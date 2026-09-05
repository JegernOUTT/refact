import { describe, expect, it } from "vitest";
import { MessageSquare, Send } from "lucide-react";
import {
  describeQueuedItem,
  queueModeOptions,
  queueStatusText,
} from "./queuePresentation";
import type {
  QueuedEvent,
  QueuedItem,
} from "../../services/refact/chatSubscription";

function deliveryItem(overrides: Partial<QueuedItem> = {}): QueuedItem {
  return {
    client_request_id: "delivery-1",
    command_type: "delivery",
    priority: false,
    preview: "Job done",
    source: "agents.push",
    enqueued_at_ms: 1,
    ...overrides,
  };
}

describe("describeQueuedItem", () => {
  it("falls back to the delivery presentation when the event carries no subkind", () => {
    const legacyEvent = {
      kind: "delivery",
      id: "delivery-1",
      source: "agents.push",
      push: "append",
      wake: false,
      message_count: 1,
    } as unknown as QueuedEvent;

    const presentation = describeQueuedItem(
      deliveryItem({ event: legacyEvent }),
    );

    expect(presentation.title).toBe("Agent continuation");
    expect(presentation.icon).toBe(Send);
    expect(presentation.tone).toBe("default");
    expect(presentation.preview).toBe("Job done");
  });

  it("keeps the agent completion title for a well-formed system notice", () => {
    const presentation = describeQueuedItem(
      deliveryItem({
        event: {
          subkind: "system_notice",
          source: "agents.push",
          payload: { status: "completed" },
        },
      }),
    );

    expect(presentation.title).toBe("Agent completed");
    expect(presentation.tone).toBe("success");
  });

  it("labels a well-formed non-agent event by its subkind", () => {
    const presentation = describeQueuedItem(
      deliveryItem({
        event: { subkind: "process_completed", source: "exec" },
        source: "exec",
      }),
    );

    expect(presentation.title).toBe("Process finished");
    expect(presentation.source).toBe("Exec");
  });

  it("falls back to the title when neither preview nor content has text", () => {
    const presentation = describeQueuedItem(
      deliveryItem({ preview: "   ", content: "", event: undefined }),
    );

    expect(presentation.preview).toBe("Agent continuation");
  });

  it("keeps the user message presentation for legacy queued messages", () => {
    const presentation = describeQueuedItem(
      deliveryItem({
        command_type: "user_message",
        content: "hello",
        source: undefined,
      }),
    );

    expect(presentation.title).toBe("Your message");
    expect(presentation.icon).toBe(MessageSquare);
    expect(presentation.isUserMessage).toBe(true);
    expect(presentation.isDelivery).toBe(false);
  });
});

const statusItem: QueuedItem = {
  client_request_id: "id",
  command_type: "delivery",
  priority: false,
  preview: "",
  enqueued_at_ms: 1,
};
describe("queue presentation", () => {
  it("summarizes busy and idle delivery timing", () => {
    const items = ["preempt", "append", "when_idle"].map(
      (push) => ({ ...statusItem, push }) as QueuedItem,
    );
    expect(queueStatusText(items, true)).toBe(
      "1 interrupting · 1 after step · 1 when idle",
    );
    expect(queueStatusText(items, false)).toBe(
      "1 interrupting · 1 ready to deliver · 1 when idle",
    );
  });
  it("keeps legacy priority options distinct", () => {
    expect(queueModeOptions(true).map((o) => o.label)).toEqual([
      "Send next",
      "In order",
    ]);
    expect(queueModeOptions(false).map((o) => o.shortLabel)).toEqual([
      "Interrupt now",
      "After step",
      "When idle",
    ]);
  });
  it("extracts process navigation and agent labels", () => {
    expect(
      describeQueuedItem({
        ...statusItem,
        event: {
          subkind: "process_completed",
          source: "exec.process",
          payload: { process_id: "proc" },
        },
      }).processId,
    ).toBe("proc");
    expect(
      describeQueuedItem({
        ...statusItem,
        source: "agents.push",
        event: {
          subkind: "system_notice",
          source: "agents.push",
          payload: { status: "ok" },
        },
      }).title,
    ).toBe("Agent completed");
    expect(
      describeQueuedItem({
        ...statusItem,
        command_type: "user_message",
        priority: true,
      }).push,
    ).toBe("preempt");
  });
});
