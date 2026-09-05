import { describe, expect, it } from "vitest";
import { MessageSquare, Send } from "lucide-react";
import { describeQueuedItem } from "./queuePresentation";
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
