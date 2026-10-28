import { useMemo } from "react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import type { ToolCall, ToolResult } from "../../../services/refact/types";
import type { ToolStatus } from "./ToolCard";
import { fromToolTone, type TranscriptStatus } from "../TranscriptItem";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";

function parseJsonOrNull(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

export function toolNameLabel(name: string): string {
  const stripped = name.startsWith("openai_")
    ? name.slice("openai_".length)
    : name;
  const words = stripped.replace(/_/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export type OpenAiResponsesToolCardState = {
  toolName: string;
  label: string;
  status: ToolStatus;
  transcriptStatus: TranscriptStatus;
  statusLabel?: string;
  parsedArgs: unknown;
  rawJson: string;
  maybeResult: ToolResult | undefined;
  contentText: string | null;
};

export function useOpenAiResponsesToolCardState(
  toolCall: ToolCall,
): OpenAiResponsesToolCardState {
  const threadId = useThreadId();

  const unrecorded = useUnrecordedResultState(toolCall.id);

  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const toolName = toolCall.function.name ?? "openai";
  const label = useMemo(() => toolNameLabel(toolName), [toolName]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult)
      return unrecorded.status === "running" ? "running" : "idle";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult, unrecorded.status]);
  const transcriptStatus: TranscriptStatus = maybeResult
    ? fromToolTone(status)
    : unrecorded.status;
  const statusLabel = maybeResult ? undefined : unrecorded.statusLabel;

  const parsedArgs = useMemo(
    () => parseJsonOrNull(toolCall.function.arguments),
    [toolCall.function.arguments],
  );

  const rawJson = useMemo(() => {
    if (parsedArgs == null) return toolCall.function.arguments;
    try {
      return JSON.stringify(parsedArgs, null, 2);
    } catch {
      return toolCall.function.arguments;
    }
  }, [parsedArgs, toolCall.function.arguments]);

  const contentText =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  return {
    toolName,
    label,
    status,
    transcriptStatus,
    statusLabel,
    parsedArgs,
    rawJson,
    maybeResult,
    contentText,
  };
}
