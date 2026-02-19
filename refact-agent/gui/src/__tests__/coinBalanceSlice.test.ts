import { describe, it, expect } from "vitest";
import { configureStore } from "@reduxjs/toolkit";
import {
  coinBallanceSlice,
  selectBalance,
} from "../features/CoinBalance/coinBalanceSlice";
import { applyChatEvent } from "../features/Chat/Thread/actions";
import type { ChatEventEnvelope } from "../services/refact/chatSubscription";

function createTestStore() {
  return configureStore({
    reducer: {
      coins: coinBallanceSlice.reducer,
    },
  });
}

describe("coinBalanceSlice", () => {
  describe("extractMeteringBalance from SSE events", () => {
    it("should extract metering_balance from stream_delta merge_extra ops", () => {
      const store = createTestStore();

      const event: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "1",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [
          { op: "append_content", text: "Hello" },
          { op: "merge_extra", extra: { metering_balance: 2553194 } },
        ],
      };
      store.dispatch(applyChatEvent(event));

      expect(selectBalance({ coins: store.getState().coins })).toBe(2553194);
    });

    it("should extract metering_balance from stream_delta merge_extra (top level simulation)", () => {
      const store = createTestStore();

      // metering_balance comes via merge_extra op, not top-level event field
      const event: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "1",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "merge_extra", extra: { metering_balance: 1000000 } }],
      };
      store.dispatch(applyChatEvent(event));

      expect(selectBalance({ coins: store.getState().coins })).toBe(1000000);
    });

    it("should extract metering_balance from stream_delta with usage in merge_extra", () => {
      const store = createTestStore();

      // Usage with metering_balance comes via merge_extra
      const event: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "1",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [
          {
            op: "merge_extra",
            extra: {
              usage: {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
              },
              metering_balance: 999999,
            },
          },
        ],
      };
      store.dispatch(applyChatEvent(event));

      expect(selectBalance({ coins: store.getState().coins })).toBe(999999);
    });

    it("should not update balance if metering_balance is missing", () => {
      const store = createTestStore();

      // Set initial balance
      const event1: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "1",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "merge_extra", extra: { metering_balance: 5000 } }],
      };
      store.dispatch(applyChatEvent(event1));

      // Event without metering_balance should not change it
      const event2: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "2",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "append_content", text: "More text" }],
      };
      store.dispatch(applyChatEvent(event2));

      expect(selectBalance({ coins: store.getState().coins })).toBe(5000);
    });

    it("should update balance with latest value from multiple ops", () => {
      const store = createTestStore();

      const event: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "1",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [
          { op: "merge_extra", extra: { metering_balance: 1000 } },
          { op: "merge_extra", extra: { other_field: "value" } },
        ],
      };
      store.dispatch(applyChatEvent(event));

      // First merge_extra with metering_balance wins
      expect(selectBalance({ coins: store.getState().coins })).toBe(1000);
    });

    it("should handle merge_extra with other metering fields but no balance", () => {
      const store = createTestStore();

      // Set initial balance
      const event1: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "1",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [{ op: "merge_extra", extra: { metering_balance: 5000 } }],
      };
      store.dispatch(applyChatEvent(event1));

      // Event with other metering fields but no balance
      const event2: ChatEventEnvelope = {
        chat_id: "test-chat",
        seq: "2",
        type: "stream_delta",
        message_id: "msg-1",
        ops: [
          {
            op: "merge_extra",
            extra: {
              metering_prompt_tokens_n: 100,
              metering_generated_tokens_n: 50,
            },
          },
        ],
      };
      store.dispatch(applyChatEvent(event2));

      // Balance should remain unchanged
      expect(selectBalance({ coins: store.getState().coins })).toBe(5000);
    });
  });
});
