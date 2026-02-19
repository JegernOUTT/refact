import {
  ChatMessage,
  ChatMessages,
  MeteringUsd,
} from "../services/refact/types";
import type { Usage } from "../services/refact/chat";

type MessageWithExtra = ChatMessage & {
  extra?: Record<string, unknown>;
};

function parseNumberish(v: unknown): number | undefined {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string") {
    const n = Number(v);
    if (Number.isFinite(n)) return n;
  }
  return undefined;
}

function getMeteringValue(
  message: MessageWithExtra,
  field: string,
): number | undefined {
  const directValue = (message as unknown as Record<string, unknown>)[field];
  const directNum = parseNumberish(directValue);
  if (directNum !== undefined) return directNum;

  const extraNum = parseNumberish(message.extra?.[field]);
  if (extraNum !== undefined) return extraNum;

  return undefined;
}

function hasCoinMetering(message: ChatMessage): boolean {
  const m = message as MessageWithExtra;
  return (
    getMeteringValue(m, "metering_coins_prompt") !== undefined ||
    getMeteringValue(m, "metering_coins_generated") !== undefined ||
    getMeteringValue(m, "metering_coins_cache_creation") !== undefined ||
    getMeteringValue(m, "metering_coins_cache_read") !== undefined
  );
}

function hasTokenMetering(message: ChatMessage): boolean {
  const m = message as MessageWithExtra;
  return (
    getMeteringValue(m, "metering_prompt_tokens_n") !== undefined ||
    getMeteringValue(m, "metering_generated_tokens_n") !== undefined ||
    getMeteringValue(m, "metering_cache_creation_tokens_n") !== undefined ||
    getMeteringValue(m, "metering_cache_read_tokens_n") !== undefined
  );
}

export function getTotalCostMeteringForMessages(messages: ChatMessages) {
  const meteringMessages = messages.filter(hasCoinMetering);
  if (meteringMessages.length === 0) return null;

  return meteringMessages.reduce<{
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
  const meteringMessages = messages.filter(hasTokenMetering);
  if (meteringMessages.length === 0) return null;

  return meteringMessages.reduce<{
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

type MessageWithUsage = ChatMessage & { usage?: Usage };

function hasUsdMetering(message: ChatMessage): boolean {
  const m = message as MessageWithUsage;
  return m.usage?.metering_usd !== undefined;
}

export function getTotalUsdMeteringForMessages(
  messages: ChatMessages,
): MeteringUsd | null {
  const meteringMessages = messages.filter(hasUsdMetering);
  if (meteringMessages.length === 0) return null;

  return meteringMessages.reduce<MeteringUsd>(
    (acc, message) => {
      const usd = (message as MessageWithUsage).usage?.metering_usd;
      if (!usd) return acc;
      return {
        prompt_usd: acc.prompt_usd + usd.prompt_usd,
        generated_usd: acc.generated_usd + usd.generated_usd,
        cache_read_usd:
          (acc.cache_read_usd ?? 0) + (usd.cache_read_usd ?? 0) || undefined,
        cache_creation_usd:
          (acc.cache_creation_usd ?? 0) + (usd.cache_creation_usd ?? 0) ||
          undefined,
        total_usd: acc.total_usd + usd.total_usd,
      };
    },
    { prompt_usd: 0, generated_usd: 0, total_usd: 0 },
  );
}

export function formatUsd(value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value)) return "–";
  if (value >= 0.01) return `$${value.toFixed(2)}`;
  if (value >= 0.001) return `$${value.toFixed(3)}`;
  if (value > 0) return `$${value.toFixed(4)}`;
  return "$0.00";
}
