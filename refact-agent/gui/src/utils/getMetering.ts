import { Usage } from "../services/refact/chat";
import {
  AssistantMessage,
  ChatMessage,
  ChatMessages,
  isAssistantMessage,
} from "../services/refact/types";

type MessageWithExtra = ChatMessage & {
  extra?: Record<string, unknown>;
};

function getMeteringValue(
  message: MessageWithExtra,
  field: string,
): number | undefined {
  const directValue = (message as unknown as Record<string, unknown>)[field];
  if (typeof directValue === "number") return directValue;

  const extraValue = message.extra?.[field];
  if (typeof extraValue === "number") return extraValue;

  return undefined;
}

export function getTotalCostMeteringForMessages(messages: ChatMessages) {
  const assistantMessages = messages.filter(hasUsageAndPrice);
  if (assistantMessages.length === 0) return null;

  return assistantMessages.reduce<{
    metering_coins_prompt: number;
    metering_coins_generated: number;
    metering_coins_cache_creation: number;
    metering_coins_cache_read: number;
  }>(
    (acc, message) => {
      return {
        metering_coins_prompt:
          acc.metering_coins_prompt +
          (getMeteringValue(message, "metering_coins_prompt") ?? 0),
        metering_coins_generated:
          acc.metering_coins_generated +
          (getMeteringValue(message, "metering_coins_generated") ?? 0),
        metering_coins_cache_creation:
          acc.metering_coins_cache_creation +
          (getMeteringValue(message, "metering_coins_cache_creation") ?? 0),
        metering_coins_cache_read:
          acc.metering_coins_cache_read +
          (getMeteringValue(message, "metering_coins_cache_read") ?? 0),
      };
    },
    {
      metering_coins_prompt: 0,
      metering_coins_generated: 0,
      metering_coins_cache_creation: 0,
      metering_coins_cache_read: 0,
    },
  );
}

export function getTotalTokenMeteringForMessages(messages: ChatMessages) {
  const assistantMessages = messages.filter(hasUsageAndPrice);
  if (assistantMessages.length === 0) return null;

  return assistantMessages.reduce<{
    metering_prompt_tokens_n: number;
    metering_generated_tokens_n: number;
    metering_cache_creation_tokens_n: number;
    metering_cache_read_tokens_n: number;
  }>(
    (acc, message) => {
      return {
        metering_prompt_tokens_n:
          acc.metering_prompt_tokens_n +
          (getMeteringValue(message, "metering_prompt_tokens_n") ?? 0),
        metering_generated_tokens_n:
          acc.metering_generated_tokens_n +
          (getMeteringValue(message, "metering_generated_tokens_n") ?? 0),
        metering_cache_creation_tokens_n:
          acc.metering_cache_creation_tokens_n +
          (getMeteringValue(message, "metering_cache_creation_tokens_n") ?? 0),
        metering_cache_read_tokens_n:
          acc.metering_cache_read_tokens_n +
          (getMeteringValue(message, "metering_cache_read_tokens_n") ?? 0),
      };
    },
    {
      metering_prompt_tokens_n: 0,
      metering_generated_tokens_n: 0,
      metering_cache_creation_tokens_n: 0,
      metering_cache_read_tokens_n: 0,
    },
  );
}
function hasUsageAndPrice(message: ChatMessage): message is AssistantMessage & {
  usage: Usage & {
    completion_tokens: number;
    prompt_tokens: number;
    cache_creation_input_tokens?: number;
    cache_read_input_tokens?: number;
  };
} {
  if (!isAssistantMessage(message)) return false;
  if (!("usage" in message)) return false;
  if (!message.usage) return false;
  if (typeof message.usage.completion_tokens !== "number") return false;
  if (typeof message.usage.prompt_tokens !== "number") return false;

  const hasCoinPrompt =
    getMeteringValue(message as MessageWithExtra, "metering_coins_prompt") !==
    undefined;
  const hasCoinGenerated =
    getMeteringValue(message as MessageWithExtra, "metering_coins_generated") !==
    undefined;
  const hasCoinCacheCreation =
    getMeteringValue(
      message as MessageWithExtra,
      "metering_coins_cache_creation",
    ) !== undefined;
  const hasCoinCacheRead =
    getMeteringValue(message as MessageWithExtra, "metering_coins_cache_read") !==
    undefined;

  if (!hasCoinPrompt) return false;
  if (!hasCoinGenerated) return false;
  if (!hasCoinCacheCreation) return false;
  if (!hasCoinCacheRead) return false;

  return true;
}
