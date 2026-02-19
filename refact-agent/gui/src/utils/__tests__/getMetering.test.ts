import { describe, it, expect } from "vitest";
import {
  getTotalCostMeteringForMessages,
  getTotalTokenMeteringForMessages,
} from "../getMetering";
import type { ChatMessages } from "../../services/refact/types";

type MeteringExtra = {
  metering_coins_prompt?: number | string;
  metering_coins_generated?: number | string;
  metering_coins_cache_creation?: number | string;
  metering_coins_cache_read?: number | string;
  metering_prompt_tokens_n?: number;
  metering_generated_tokens_n?: number;
  metering_cache_creation_tokens_n?: number;
  metering_cache_read_tokens_n?: number;
};

type MessageWithExtra = {
  role: "assistant";
  content: string;
  usage?: {
    completion_tokens: number;
    prompt_tokens: number;
    total_tokens: number;
  };
  tool_calls?: {
    id: string;
    function: { name: string; arguments: string };
    index: number;
  }[];
  metering_coins_prompt?: number;
  metering_coins_generated?: number;
  metering_coins_cache_creation?: number;
  metering_coins_cache_read?: number;
  extra?: MeteringExtra;
};

type ToolMessageWithExtra = {
  role: "tool";
  content: string;
  tool_call_id: string;
  extra?: MeteringExtra;
};

describe("getMetering", () => {
  describe("getTotalCostMeteringForMessages", () => {
    it("should extract metering from message.extra (new format)", () => {
      const messages = [
        { role: "user", content: "Hello" },
        {
          role: "assistant",
          content: "Hi there",
          usage: {
            completion_tokens: 10,
            prompt_tokens: 20,
            total_tokens: 10 + 20,
          },
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 100,
        metering_coins_generated: 50,
        metering_coins_cache_creation: 0,
        metering_coins_cache_read: 0,
      });
    });

    it("should extract metering from direct properties (legacy format)", () => {
      const messages = [
        { role: "user", content: "Hello" },
        {
          role: "assistant",
          content: "Hi there",
          usage: {
            completion_tokens: 10,
            prompt_tokens: 20,
            total_tokens: 10 + 20,
          },
          metering_coins_prompt: 200,
          metering_coins_generated: 100,
          metering_coins_cache_creation: 10,
          metering_coins_cache_read: 5,
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 200,
        metering_coins_generated: 100,
        metering_coins_cache_creation: 10,
        metering_coins_cache_read: 5,
      });
    });

    it("should prefer direct properties over extra (backward compatibility)", () => {
      const messages = [
        {
          role: "assistant",
          content: "Test",
          usage: {
            completion_tokens: 10,
            prompt_tokens: 20,
            total_tokens: 10 + 20,
          },
          metering_coins_prompt: 300,
          metering_coins_generated: 150,
          metering_coins_cache_creation: 20,
          metering_coins_cache_read: 10,
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 300,
        metering_coins_generated: 150,
        metering_coins_cache_creation: 20,
        metering_coins_cache_read: 10,
      });
    });

    it("should aggregate metering from multiple messages", () => {
      const messages = [
        {
          role: "assistant",
          content: "First",
          usage: {
            completion_tokens: 10,
            prompt_tokens: 20,
            total_tokens: 10 + 20,
          },
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } satisfies MessageWithExtra,
        { role: "user", content: "Follow up" },
        {
          role: "assistant",
          content: "Second",
          usage: {
            completion_tokens: 15,
            prompt_tokens: 25,
            total_tokens: 15 + 25,
          },
          extra: {
            metering_coins_prompt: 150,
            metering_coins_generated: 75,
            metering_coins_cache_creation: 10,
            metering_coins_cache_read: 5,
          },
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 250,
        metering_coins_generated: 125,
        metering_coins_cache_creation: 10,
        metering_coins_cache_read: 5,
      });
    });

    it("should return null when no messages have metering data", () => {
      const messages = [
        { role: "user", content: "Hello" },
        { role: "assistant", content: "Hi" },
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toBeNull();
    });

    it("should return null for empty messages array", () => {
      const result = getTotalCostMeteringForMessages([]);
      expect(result).toBeNull();
    });

    it("should extract metering from tool messages (subagent results)", () => {
      const messages = [
        { role: "user", content: "Hello" },
        {
          role: "assistant",
          content: "Let me delegate this",
          tool_calls: [
            {
              id: "call_123",
              function: { name: "subagent", arguments: "{}" },
              index: 0,
            },
          ],
        } satisfies MessageWithExtra,
        {
          role: "tool",
          content: "Subagent result",
          tool_call_id: "call_123",
          extra: {
            metering_coins_prompt: 500,
            metering_coins_generated: 250,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } satisfies ToolMessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 500,
        metering_coins_generated: 250,
        metering_coins_cache_creation: 0,
        metering_coins_cache_read: 0,
      });
    });

    it("should aggregate metering from both assistant and tool messages", () => {
      const messages = [
        {
          role: "assistant",
          content: "First response",
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } satisfies MessageWithExtra,
        {
          role: "tool",
          content: "Tool result",
          tool_call_id: "call_123",
          extra: {
            metering_coins_prompt: 200,
            metering_coins_generated: 100,
            metering_coins_cache_creation: 5,
            metering_coins_cache_read: 3,
          },
        } satisfies ToolMessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 300,
        metering_coins_generated: 150,
        metering_coins_cache_creation: 5,
        metering_coins_cache_read: 3,
      });
    });

    it("should handle string numbers from providers", () => {
      const messages = [
        {
          role: "assistant",
          content: "Test",
          extra: {
            metering_coins_prompt: "100.5",
            metering_coins_generated: "50.25",
            metering_coins_cache_creation: "0",
            metering_coins_cache_read: "0",
          },
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 100.5,
        metering_coins_generated: 50.25,
        metering_coins_cache_creation: 0,
        metering_coins_cache_read: 0,
      });
    });
  });

  describe("getTotalTokenMeteringForMessages", () => {
    it("should extract token metering from message.extra", () => {
      const messages = [
        {
          role: "assistant",
          content: "Test",
          usage: {
            completion_tokens: 10,
            prompt_tokens: 20,
            total_tokens: 10 + 20,
          },
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
            metering_prompt_tokens_n: 1000,
            metering_generated_tokens_n: 500,
            metering_cache_creation_tokens_n: 0,
            metering_cache_read_tokens_n: 0,
          },
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalTokenMeteringForMessages(messages);

      expect(result).toEqual({
        metering_prompt_tokens_n: 1000,
        metering_generated_tokens_n: 500,
        metering_cache_creation_tokens_n: 0,
        metering_cache_read_tokens_n: 0,
      });
    });

    it("should return null when no messages have token metering", () => {
      const messages = [
        {
          role: "assistant",
          content: "Test",
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } satisfies MessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalTokenMeteringForMessages(messages);

      expect(result).toBeNull();
    });

    it("should extract token metering from tool messages", () => {
      const messages = [
        {
          role: "tool",
          content: "Subagent result",
          tool_call_id: "call_123",
          extra: {
            metering_prompt_tokens_n: 2000,
            metering_generated_tokens_n: 1000,
            metering_cache_creation_tokens_n: 100,
            metering_cache_read_tokens_n: 50,
          },
        } satisfies ToolMessageWithExtra,
      ] as unknown as ChatMessages;

      const result = getTotalTokenMeteringForMessages(messages);

      expect(result).toEqual({
        metering_prompt_tokens_n: 2000,
        metering_generated_tokens_n: 1000,
        metering_cache_creation_tokens_n: 100,
        metering_cache_read_tokens_n: 50,
      });
    });
  });
});
