import React, { useMemo } from "react";
import { GearIcon } from "@radix-ui/react-icons";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import {
  selectToolResultById,
  selectIsStreaming,
  selectIsWaiting,
} from "../../../features/Chat/Thread/selectors";
import type { ToolCall } from "../../../services/refact/types";
import { formatToolDisplayName } from "../../../utils/toolNameAliases";
import styles from "./GenericTool.module.css";

interface GenericToolProps {
  toolCall: ToolCall;
}

function formatArgs(argsStr: string): string {
  try {
    const args = JSON.parse(argsStr) as Record<string, unknown>;
    const entries = Object.entries(args);
    if (entries.length === 0) return "";
    return entries
      .map(([k, v]) => {
        const valueStr = typeof v === "string" ? v : JSON.stringify(v);
        return `${k}=${valueStr}`;
      })
      .join(", ");
  } catch {
    return argsStr;
  }
}

export const GenericTool: React.FC<GenericToolProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

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

  const toolName = toolCall.function.name ?? "tool";
  const argsPreview = formatArgs(toolCall.function.arguments);

  const summary = useMemo(() => {
    const displayName = formatToolDisplayName(toolName);
    if (argsPreview) {
      return (
        <>
          {displayName} <span className={styles.args}>{argsPreview}</span>
        </>
      );
    }
    return displayName;
  }, [toolName, argsPreview]);

  return (
    <>
      <span data-testid="generic-tool" hidden />
      <ToolCard
        icon={<GearIcon />}
        summary={summary}
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      />
    </>
  );
};

export default GenericTool;
