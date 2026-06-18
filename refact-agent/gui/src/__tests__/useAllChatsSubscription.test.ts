import { describe, it, expect } from "vitest";
import { pickDesiredChatSubscriptions } from "../hooks/useAllChatsSubscription";
import {
  connectionSlice,
  registerVisibleChatMount,
  unregisterVisibleChatMount,
  selectVisibleChatMountIds,
} from "../features/Connection";
import type { RootState } from "../app/store";

describe("pickDesiredChatSubscriptions", () => {
  it("subscribes only chats visible on screen", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("deduplicates visible chats while preserving order", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-2", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("does not keep chat subscriptions while the page is hidden", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-1", "chat-2"],
      documentVisible: false,
    });

    expect(result).toEqual([]);
  });

  it("derives desired subscriptions from registered visible chat mounts", () => {
    let connection = connectionSlice.reducer(
      undefined,
      registerVisibleChatMount({ chatId: "chat-a" }),
    );
    connection = connectionSlice.reducer(
      connection,
      registerVisibleChatMount({ chatId: "chat-b" }),
    );
    connection = connectionSlice.reducer(
      connection,
      registerVisibleChatMount({ chatId: "chat-b" }),
    );
    connection = connectionSlice.reducer(
      connection,
      unregisterVisibleChatMount({ chatId: "chat-a" }),
    );

    const visibleThreadIds = selectVisibleChatMountIds({
      connection,
    } as unknown as RootState);
    const result = pickDesiredChatSubscriptions({ visibleThreadIds });

    expect(result).toEqual(["chat-b"]);
  });
});
