import { CircleCheck, TriangleAlert, Timer } from "lucide-react";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Flex, Text } from "@radix-ui/themes";
import { Button } from "../../ui";

import { ToolCard } from "./ToolCard";
import type { ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useChatActions } from "../../../hooks/useChatActions";
import { selectMessagesById } from "../../../features/Chat/Thread/selectors";
import { useToolCallActivity } from "./useToolCallActivity";
import { selectConfig } from "../../../features/Config/configSlice";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import type {
  EventMessage,
  ToolCall,
  ToolResult,
} from "../../../services/refact/types";
import { normalizeEventMessageMetadata } from "../../../services/refact/types";
import { CompactToolDetails } from "../../Tools/AgenticTool/CompactToolDetails";
import { compactText } from "../../Tools/AgenticTool/compactPresentation";
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
  if (
    !parsed ||
    typeof parsed.slept_ms !== "number" ||
    !Number.isFinite(parsed.slept_ms) ||
    parsed.slept_ms < 0 ||
    typeof parsed.interrupted !== "boolean"
  )
    return null;
  return {
    sleptMs: parsed.slept_ms,
    interrupted: parsed.interrupted,
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
  const [isOpen, handleToggle] = useStoredOpen(storeKey, false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const {
    result: resultMessage,
    isRunning,
    threadId: resolvedThreadId,
  } = useToolCallActivity(toolCall.id, threadId);
  const { abort } = useChatActions(resolvedThreadId);
  const config = useAppSelector(selectConfig);
  const endpointIdentity = getEngineEndpointIdentity(config);
  const source = useMemo(
    () => ({
      resolvedThreadId,
      toolCallId: toolCall.id,
      endpointIdentity,
      apiKey: config.apiKey,
      isRunning,
    }),
    [resolvedThreadId, toolCall.id, endpointIdentity, config.apiKey, isRunning],
  );
  const sourceRef = useRef<object | null>(source);
  sourceRef.current = source;
  const [wakeState, setWakeState] = useState<{
    source: object;
    pending?: boolean;
    error?: string;
  } | null>(null);
  const currentWakeState = wakeState?.source === source ? wakeState : null;
  useEffect(() => {
    sourceRef.current = source;
    return () => {
      sourceRef.current = null;
    };
  }, [source]);

  const sleepArgs = useMemo(() => parseSleepArgs(toolCall), [toolCall]);
  const messages = useAppSelector((state) =>
    selectMessagesById(state, resolvedThreadId),
  );
  const sleepResult = useMemo(
    () => parseSleepResult(resultMessage),
    [resultMessage],
  );
  const status: ToolStatus = isRunning
    ? "running"
    : resultMessage?.tool_failed
      ? "error"
      : sleepResult
        ? "success"
        : "idle";
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
    () =>
      isRunning ? resolveSleepStartedAtMs(resolvedThreadId, toolCall.id) : 0,
    [isRunning, resolvedThreadId, toolCall.id],
  );

  useEffect(() => {
    if (!isRunning) return;
    const interval = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(interval);
  }, [isRunning]);

  useEffect(() => {
    if (isRunning) return;
    removeStoredSleepStartedAt(resolvedThreadId, toolCall.id);
  }, [isRunning, resolvedThreadId, toolCall.id]);

  const fallbackRemainingMs = Math.max(
    0,
    sleepArgs.durationMs - Math.max(0, nowMs - startedAtMs),
  );
  const remainingMs =
    remainingFromTick(sleepArgs.durationMs, ticks) ?? fallbackRemainingMs;

  const stateLabel = isRunning
    ? `${formatSeconds(remainingMs)} remaining`
    : resultMessage?.tool_failed
      ? "Wait request failed"
      : sleepResult?.interrupted
        ? "Interrupted"
        : sleepResult
          ? "Wait ended"
          : resultMessage
            ? "Outcome unknown"
            : "Result not recorded";
  const facts = [
    { label: "Requested wait", value: formatSeconds(sleepArgs.durationMs) },
    ...(sleepResult
      ? [
          {
            label: sleepResult.interrupted ? "Interrupted after" : "Slept",
            value: formatCompletedSeconds(sleepResult.sleptMs),
          },
        ]
      : []),
    { label: "Recorded ticks", value: String(ticks.length) },
  ];

  const handleWakeUp = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (!isRunning || currentWakeState?.pending) return;
      setWakeState({ source, pending: true });
      void abort(resolvedThreadId).then(
        () => {
          if (sourceRef.current === source) setWakeState(null);
        },
        (cause: unknown) => {
          if (sourceRef.current !== source) return;
          setWakeState({
            source,
            error: cause instanceof Error ? cause.message : String(cause),
          });
        },
      );
    },
    [abort, currentWakeState?.pending, isRunning, resolvedThreadId, source],
  );

  const icon =
    sleepResult?.interrupted === true || resultMessage?.tool_failed === true ? (
      <span className={styles.summaryInterrupted}>
        <TriangleAlert />
      </span>
    ) : sleepResult ? (
      <span className={styles.summarySuccess}>
        <CircleCheck />
      </span>
    ) : (
      <Timer />
    );

  return (
    <div data-testid="sleep-tool-card">
      <ToolCard
        icon={icon}
        summary={compactText(sleepArgs.description, 160) ?? "Wait briefly"}
        meta={stateLabel}
        density="compact"
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
        className={styles.sleepCard}
      >
        {isRunning && (
          <Flex direction="column" gap="1">
            <Button
              type="button"
              variant="soft"
              disabled={currentWakeState?.pending}
              onClick={handleWakeUp}
            >
              <Timer />
              Wake up (abort chat)
            </Button>
            <Text size="1" color="gray">
              Aborts the whole chat turn, not only this wait.
            </Text>
            {currentWakeState?.error && (
              <Text size="1" color="red" role="alert">
                {currentWakeState.error}
              </Text>
            )}
          </Flex>
        )}
        <CompactToolDetails facts={facts}>
          <Flex direction="column" gap="2" className={styles.countdown}>
            <Flex align="center" justify="between" gap="2" wrap="wrap">
              <Flex direction="column" gap="1">
                <Text weight="bold" className={styles.countdownText}>
                  {isRunning
                    ? `Sleeping… ${formatSeconds(remainingMs)} remaining`
                    : sleepResult?.interrupted
                      ? `Interrupted after ${formatCompletedSeconds(
                          sleepResult.sleptMs,
                        )}`
                      : sleepResult
                        ? `Slept ${formatCompletedSeconds(
                            sleepResult.sleptMs,
                          )} · ${ticks.length} ticks`
                        : stateLabel}
                </Text>
                {sleepArgs.description && (
                  <Text size="1" className={styles.description}>
                    {sleepArgs.description}
                  </Text>
                )}
              </Flex>
            </Flex>
            <TickDots ticks={ticks} />
          </Flex>
        </CompactToolDetails>
      </ToolCard>
    </div>
  );
};

export default SleepToolCard;
