import { describe, it, expect } from "vitest";
import {
  getTotalCostMeteringForMessages,
  getTotalTokenMeteringForMessages,
} from "../getMetering";
import { ChatMessages } from "../../services/refact/types";

describe("getMetering", () => {
  describe("getTotalCostMeteringForMessages", () => {
    it("should extract metering from message.extra (new format)", () => {
      const messages: ChatMessages = [
        { role: "user", content: "Hello" },
        {
          role: "assistant",
          content: "Hi there",
          usage: { completion_tokens: 10, prompt_tokens: 20 },
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } as any,
      ];

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 100,
        metering_coins_generated: 50,
        metering_coins_cache_creation: 0,
        metering_coins_cache_read: 0,
      });
    });

    it("should extract metering from direct properties (legacy format)", () => {
      const messages: ChatMessages = [
        { role: "user", content: "Hello" },
        {
          role: "assistant",
          content: "Hi there",
          usage: { completion_tokens: 10, prompt_tokens: 20 },
          metering_coins_prompt: 200,
          metering_coins_generated: 100,
          metering_coins_cache_creation: 10,
          metering_coins_cache_read: 5,
        } as any,
      ];

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 200,
        metering_coins_generated: 100,
        metering_coins_cache_creation: 10,
        metering_coins_cache_read: 5,
      });
    });

    it("should prefer direct properties over extra (backward compatibility)", () => {
      const messages: ChatMessages = [
        {
          role: "assistant",
          content: "Test",
          usage: { completion_tokens: 10, prompt_tokens: 20 },
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
        } as any,
      ];

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 300,
        metering_coins_generated: 150,
        metering_coins_cache_creation: 20,
        metering_coins_cache_read: 10,
      });
    });

    it("should aggregate metering from multiple messages", () => {
      const messages: ChatMessages = [
        {
          role: "assistant",
          content: "First",
          usage: { completion_tokens: 10, prompt_tokens: 20 },
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } as any,
        { role: "user", content: "Follow up" },
        {
          role: "assistant",
          content: "Second",
          usage: { completion_tokens: 15, prompt_tokens: 25 },
          extra: {
            metering_coins_prompt: 150,
            metering_coins_generated: 75,
            metering_coins_cache_creation: 10,
            metering_coins_cache_read: 5,
          },
        } as any,
      ];

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toEqual({
        metering_coins_prompt: 250,
        metering_coins_generated: 125,
        metering_coins_cache_creation: 10,
        metering_coins_cache_read: 5,
      });
    });

    it("should return null when no messages have metering data", () => {
      const messages: ChatMessages = [
        { role: "user", content: "Hello" },
        { role: "assistant", content: "Hi" },
      ];

      const result = getTotalCostMeteringForMessages(messages);

      expect(result).toBeNull();
    });

    it("should return null for empty messages array", () => {
      const result = getTotalCostMeteringForMessages([]);
      expect(result).toBeNull();
    });
  });

  describe("getTotalTokenMeteringForMessages", () => {
    it("should extract token metering from message.extra", () => {
      const messages: ChatMessages = [
        {
          role: "assistant",
          content: "Test",
          usage: { completion_tokens: 10, prompt_tokens: 20 },
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
        } as any,
      ];

      const result = getTotalTokenMeteringForMessages(messages);

      expect(result).toEqual({
        metering_prompt_tokens_n: 1000,
        metering_generated_tokens_n: 500,
        metering_cache_creation_tokens_n: 0,
        metering_cache_read_tokens_n: 0,
      });
    });

    it("should handle missing token fields gracefully", () => {
      const messages: ChatMessages = [
        {
          role: "assistant",
          content: "Test",
          usage: { completion_tokens: 10, prompt_tokens: 20 },
          extra: {
            metering_coins_prompt: 100,
            metering_coins_generated: 50,
            metering_coins_cache_creation: 0,
            metering_coins_cache_read: 0,
          },
        } as any,
      ];

      const result = getTotalTokenMeteringForMessages(messages);

      expect(result).toEqual({
        metering_prompt_tokens_n: 0,
        metering_generated_tokens_n: 0,
        metering_cache_creation_tokens_n: 0,
        metering_cache_read_tokens_n: 0,
      });
    });
  });
});
