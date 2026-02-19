import { useMemo } from "react";
import { useStoredOpen } from "../useStoredOpen";

import { useAppSelector } from "../../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectToolResultById,
} from "../../../features/Chat/Thread/selectors";
import type { ToolCall, ToolResult } from "../../../services/refact/types";
import type { ToolStatus } from "./ToolCard";

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
  return stripped.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

export type OpenAiResponsesToolCardState = {
  toolName: string;
  label: string;
  isOpen: boolean;
  toggleOpen: () => void;
  status: ToolStatus;
  parsedArgs: unknown;
  rawJson: string;
  maybeResult: ToolResult | undefined;
  contentText: string | null;
};

export function useOpenAiResponsesToolCardState(
  toolCall: ToolCall,
): OpenAiResponsesToolCardState {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, toggleOpen] = useStoredOpen(storeKey, false);

  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const toolName = toolCall.function.name ?? "openai";
  const label = useMemo(() => toolNameLabel(toolName), [toolName]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult, isStreaming, isWaiting]);

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
    isOpen,
    toggleOpen,
    status,
    parsedArgs,
    rawJson,
    maybeResult,
    contentText,
  };
}
