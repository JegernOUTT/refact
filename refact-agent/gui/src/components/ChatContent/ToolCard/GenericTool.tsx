import React, { useMemo } from "react";
import { GearIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import {
  selectToolResultById,
  selectIsStreaming,
  selectIsWaiting,
} from "../../../features/Chat/Thread/selectors";
import type { ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { Markdown } from "../../Markdown";
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

function looksLikeMarkdown(text: string): boolean {
  if (text.includes("```")) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(text)) return true;
  if (/^#{1,6}\s+\S/m.test(text)) return true;
  if (/^\s*([-*+])\s+\S/m.test(text)) return true;
  if (/^\s*\d+\.\s+\S/m.test(text)) return true;
  const hasTableHeader = /^\s*\|.+\|\s*$/m.test(text);
  const hasTableSep = /^\s*\|[\s:|-]+\|\s*$/m.test(text);
  if (hasTableHeader && hasTableSep) return true;
  return false;
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

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

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

  const shouldRenderMarkdown =
    content && content.length <= 50000 && looksLikeMarkdown(content);

  return (
    <div data-testid="generic-tool" style={{ display: "contents" }}>
    <ToolCard
      icon={<GearIcon />}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {content && (
        <Box className={styles.resultContent}>
          {shouldRenderMarkdown ? (
            <Box className={styles.markdownContent}>
              <Markdown>{content}</Markdown>
            </Box>
          ) : (
            <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
          )}
        </Box>
      )}
    </ToolCard>
    </div>
  );
};

export default GenericTool;
