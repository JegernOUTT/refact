import { CircleCheck, TriangleAlert, Timer } from "lucide-react";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { Button } from "../../ui";

import { ToolCard } from "./ToolCard";
import type { ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import { useChatActions } from "../../../hooks/useChatActions";
import {
  selectMessages,
  selectMessagesById,
  selectToolResultById,
  selectToolResultByThreadAndId,
} from "../../../features/Chat/Thread/selectors";
import type {
  EventMessage,
  ToolCall,
  ToolResult,
} from "../../../services/refact/types";
import { normalizeEventMessageMetadata } from "../../../services/refact/types";
import styles from "./SleepToolCard.module.css";

type SleepArgs = {
  durationMs: number;
  description?: string;
};

type SleepResult = {
  sleptMs: number;
  interrupted: boolean;
};

type SleepTick = {
  id: string;
  elapsedMs: number;
  remainingMs: number;
};

type SleepToolCardProps = {
  toolCall: ToolCall;
  threadId?: string;
};

const SLEEP_STARTED_AT_STORAGE_PREFIX = "refact.sleep.startedAt";

function parseSleepArgs(toolCall: ToolCall): SleepArgs {
  try {
    const parsed = JSON.parse(toolCall.function.arguments) as unknown;
    if (!isRecord(parsed)) return { durationMs: 0 };
    return {
      durationMs: numberField(parsed.duration_ms),
      description:
        typeof parsed.description === "string" ? parsed.description : undefined,
    };
  } catch {
    return { durationMs: 0 };
  }
}

function parseSleepResult(result: ToolResult | undefined): SleepResult | null {
  if (!result) return null;
  const extraSleep = isRecord(result.extra?.sleep) ? result.extra.sleep : null;
  const parsed = extraSleep ?? parseJsonRecord(result.content);
  if (!parsed) return null;
  return {
    sleptMs: numberField(parsed.slept_ms),
    interrupted: parsed.interrupted === true,
  };
}

function parseJsonRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value !== "string") return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function numberField(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function eventMatchesSleepTick(message: EventMessage): boolean {
  return message.subkind === "tick" && message.source === "tool.sleep";
}

function tickFromEvent(message: EventMessage, index: number): SleepTick | null {
  if (!eventMatchesSleepTick(message)) return null;
  if (!isRecord(message.payload)) return null;
  return {
    id: message.message_id ?? `sleep-tick-${index}`,
    elapsedMs: numberField(message.payload.elapsed_ms),
    remainingMs: numberField(message.payload.remaining_ms),
  };
}

function formatSeconds(ms: number): string {
  return `${Math.max(0, Math.ceil(ms / 1000))}s`;
}

function formatCompletedSeconds(ms: number): string {
  return `${Math.max(0, Math.round(ms / 1000))}s`;
}

function remainingFromTick(
  durationMs: number,
  ticks: SleepTick[],
): number | null {
  const latest = ticks.at(-1);
  if (!latest) return null;
  if (latest.remainingMs > 0) return latest.remainingMs;
  return Math.max(0, durationMs - latest.elapsedMs);
}

function statusFromResult(result: SleepResult | null): ToolStatus {
  if (!result) return "running";
  return result.interrupted ? "error" : "success";
}

function sleepStorageKey(
  threadId: string | undefined,
  toolCallId?: string,
): string | null {
  if (!toolCallId) return null;
  const threadPart = threadId ?? "current";
  return `${SLEEP_STARTED_AT_STORAGE_PREFIX}:${threadPart}:${toolCallId}`;
}

function readStoredSleepStartedAt(key: string): number | null {
  try {
    const value = window.localStorage.getItem(key);
    if (!value) return null;
    const parsed = Number(value);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  } catch {
    return null;
  }
}

function writeStoredSleepStartedAt(key: string, startedAtMs: number): void {
  try {
    window.localStorage.setItem(key, String(startedAtMs));
  } catch {
    return;
  }
}

function removeStoredSleepStartedAt(
  threadId: string | undefined,
  toolCallId?: string,
): void {
  const key = sleepStorageKey(threadId, toolCallId);
  if (!key) return;
  try {
    window.localStorage.removeItem(key);
  } catch {
    return;
  }
}

function resolveSleepStartedAtMs(
  threadId: string | undefined,
  toolCallId?: string,
): number {
  const key = sleepStorageKey(threadId, toolCallId);
  if (!key) return Date.now();
  const stored = readStoredSleepStartedAt(key);
  if (stored !== null) return stored;
  const startedAtMs = Date.now();
  writeStoredSleepStartedAt(key, startedAtMs);
  return startedAtMs;
}

const TickDots = React.memo(function TickDots({
  ticks,
}: {
  ticks: SleepTick[];
}) {
  const visibleTicks = ticks.slice(-12);
  return (
    <Flex
      align="center"
      gap="2"
      wrap="wrap"
      className={styles.tickStream}
      data-testid="sleep-tick-stream"
    >
      {visibleTicks.map((tick, index) => (
        <span
          key={tick.id}
          className={styles.tickDot}
          data-testid="sleep-tick-dot"
          data-tick-index={index}
        />
      ))}
    </Flex>
  );
});

export const SleepToolCard: React.FC<SleepToolCardProps> = ({
  toolCall,
  threadId,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const { abort } = useChatActions();

  const sleepArgs = useMemo(() => parseSleepArgs(toolCall), [toolCall]);
  const messages = useAppSelector((state) =>
    threadId === undefined
      ? selectMessages(state)
      : selectMessagesById(state, threadId),
  );
  const resultMessage = useAppSelector((state) =>
    threadId === undefined
      ? selectToolResultById(state, toolCall.id)
      : selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const sleepResult = useMemo(
    () => parseSleepResult(resultMessage),
    [resultMessage],
  );
  const status = statusFromResult(sleepResult);
  const isRunning = status === "running";
  const ticks = useMemo(
    () =>
      messages.flatMap((message, index) => {
        if (message.role !== "event") return [];
        const tick = tickFromEvent(
          normalizeEventMessageMetadata(message),
          index,
        );
        return tick ? [tick] : [];
      }),
    [messages],
  );
  const startedAtMs = useMemo(
    () => resolveSleepStartedAtMs(threadId, toolCall.id),
    [threadId, toolCall.id],
  );

  useEffect(() => {
    if (!isRunning) return;
    const interval = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(interval);
  }, [isRunning]);

  useEffect(() => {
    if (isRunning) return;
    removeStoredSleepStartedAt(threadId, toolCall.id);
  }, [isRunning, threadId, toolCall.id]);

  const fallbackRemainingMs = Math.max(
    0,
    sleepArgs.durationMs - Math.max(0, nowMs - startedAtMs),
  );
  const remainingMs =
    remainingFromTick(sleepArgs.durationMs, ticks) ?? fallbackRemainingMs;

  const summary = useMemo(() => {
    if (!sleepResult) {
      return `Sleeping… ${formatSeconds(remainingMs)} remaining`;
    }
    if (sleepResult.interrupted) {
      return (
        <span className={styles.summaryInterrupted}>
          Interrupted after {formatCompletedSeconds(sleepResult.sleptMs)}
        </span>
      );
    }
    return (
      <span className={styles.summarySuccess}>
        Slept {formatCompletedSeconds(sleepResult.sleptMs)} · {ticks.length}{" "}
        ticks
      </span>
    );
  }, [remainingMs, sleepResult, ticks.length]);

  const handleWakeUp = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      void abort(threadId);
    },
    [abort, threadId],
  );

  const icon = sleepResult?.interrupted ? (
    <span className={styles.summaryInterrupted}>
      <TriangleAlert />
    </span>
  ) : (
    <span className={styles.summarySuccess}>
      <CircleCheck />
    </span>
  );

  return (
    <div data-testid="sleep-tool-card">
      <ToolCard
        icon={icon}
        summary={summary}
        meta={!sleepResult ? `${ticks.length} ticks` : undefined}
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
        className={styles.sleepCard}
      >
        {isRunning && (
          <Flex direction="column" gap="2" className={styles.countdown}>
            <Flex align="center" justify="between" gap="2" wrap="wrap">
              <Flex direction="column" gap="1">
                <Text weight="bold" className={styles.countdownText}>
                  Sleeping… {formatSeconds(remainingMs)} remaining
                </Text>
                {sleepArgs.description && (
                  <Text size="1" className={styles.description}>
                    {sleepArgs.description}
                  </Text>
                )}
              </Flex>
              <Button type="button" variant="soft" onClick={handleWakeUp}>
                <Timer />
                Wake up
              </Button>
            </Flex>
            <TickDots ticks={ticks} />
          </Flex>
        )}
      </ToolCard>
    </div>
  );
};

export default SleepToolCard;
