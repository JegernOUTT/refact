import { describe, it, expect } from "vitest";
import { pickDesiredChatSubscriptions } from "../hooks/useAllChatsSubscription";

const chatIds = (count: number) =>
  Array.from({ length: count }, (_, index) => `chat-${index + 1}`);

describe("pickDesiredChatSubscriptions", () => {
  it("uses a default cap of 50 subscriptions", () => {
    const result = pickDesiredChatSubscriptions({
      openThreadIds: chatIds(60),
      activeChatId: "chat-60",
      subscribedThreadIds: [],
    });

    expect(result).toHaveLength(50);
    expect(result).toEqual(chatIds(60).slice(10).reverse());
  });

  it("subscribes visible pane threads first beyond the old 4 cap", () => {
    const result = pickDesiredChatSubscriptions({
      openThreadIds: chatIds(8),
      visibleThreadIds: ["chat-2", "chat-5", "chat-7", "chat-8", "chat-4"],
      activeChatId: "chat-6",
      subscribedThreadIds: [],
    });

    expect(result).toEqual([
      "chat-2",
      "chat-5",
      "chat-7",
      "chat-8",
      "chat-4",
      "chat-6",
      "chat-3",
      "chat-1",
    ]);
  });

  it("never evicts visible panes before background tabs", () => {
    const result = pickDesiredChatSubscriptions({
      openThreadIds: chatIds(10),
      visibleThreadIds: ["chat-2", "chat-9", "chat-4"],
      activeChatId: "chat-10",
      subscribedThreadIds: ["chat-1", "chat-3", "chat-5"],
      maxSubscriptions: 5,
    });

    expect(result).toEqual([
      "chat-2",
      "chat-9",
      "chat-4",
      "chat-10",
      "chat-1",
    ]);
  });

  it("preserves non-visible ordering with active then subscribed then open tabs", () => {
    const result = pickDesiredChatSubscriptions({
      openThreadIds: ["chat-1", "chat-2", "chat-3", "chat-4", "chat-5"],
      activeChatId: "chat-3",
      subscribedThreadIds: ["chat-1", "chat-2"],
      maxSubscriptions: 4,
    });

    expect(result).toEqual(["chat-3", "chat-1", "chat-2", "chat-5"]);
  });

  it("includes active chat even when it is not in open tabs", () => {
    const result = pickDesiredChatSubscriptions({
      openThreadIds: ["chat-1", "chat-2", "chat-3", "chat-4"],
      activeChatId: "chat-external",
      subscribedThreadIds: [],
      maxSubscriptions: 4,
    });

    expect(result).toEqual(["chat-external", "chat-4", "chat-3", "chat-2"]);
  });

  it("returns full ordered list when maxSubscriptions is non-positive", () => {
    const result = pickDesiredChatSubscriptions({
      openThreadIds: ["chat-1", "chat-2", "chat-3"],
      visibleThreadIds: ["chat-3"],
      activeChatId: "chat-2",
      subscribedThreadIds: ["chat-1"],
      maxSubscriptions: 0,
    });

    expect(result).toEqual(["chat-3", "chat-2", "chat-1"]);
  });
});
