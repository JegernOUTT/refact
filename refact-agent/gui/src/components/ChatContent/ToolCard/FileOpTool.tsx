import React, { useMemo, useCallback } from "react";
import { MoveIcon, TrashIcon, PlusCircledIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import {
  selectToolResultById,
  selectManyDiffMessageByIds,
  selectIsStreaming,
  selectIsWaiting,
} from "../../../features/Chat/Thread/selectors";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { basename } from "./utils";
import styles from "./FileOpTool.module.css";

type FileOpType = "mv" | "rm" | "add_workspace_folder";

interface MvArgs {
  source?: string;
  destination?: string;
}

interface RmArgs {
  path?: string;
  recursive?: boolean;
}

interface AddWorkspaceArgs {
  path?: string;
}

interface FileOpToolProps {
  toolCall: ToolCall;
  toolType: FileOpType;
  diffs?: DiffChunk[];
  isActiveTool?: boolean;
}

function countNonEmptyLines(text: string): number {
  let count = 0;
  let hasContent = false;

  for (const char of text) {
    if (char === "\n") {
      if (hasContent) count++;
      hasContent = false;
    } else if (char !== "\r" && char !== " " && char !== "\t") {
      hasContent = true;
    }
  }

  return hasContent ? count + 1 : count;
}

export const FileOpTool: React.FC<FileOpToolProps> = ({
  toolCall,
  toolType,
  diffs = [],
  isActiveTool = true,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const { queryPathThenOpenFile } = useEventsBusForIDE();
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const diffIds = useMemo(
    () => (toolCall.id ? [toolCall.id] : []),
    [toolCall.id],
  );
  const selectDiffs = useMemo(
    () => selectManyDiffMessageByIds(diffIds),
    [diffIds],
  );
  const toolDiffs = useAppSelector(selectDiffs);

  const hasResult = maybeResult !== undefined;
  const hasDiffs = diffs.length > 0 || toolDiffs.length > 0;
  const isToolBusy = isActiveTool && !hasResult && (isStreaming || isWaiting);
  const shouldReadDiffs = hasDiffs && !isToolBusy;

  const allDiffs = useMemo((): DiffChunk[] => {
    if (!shouldReadDiffs) return [];

    const fromProps = diffs;
    const fromStore = toolDiffs.flatMap((d) => d.content);
    return fromProps.length > 0 ? fromProps : fromStore;
  }, [diffs, shouldReadDiffs, toolDiffs]);

  const args = useMemo((): MvArgs | RmArgs | AddWorkspaceArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as
        | MvArgs
        | RmArgs
        | AddWorkspaceArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const status: ToolStatus = useMemo(() => {
    if (maybeResult) {
      if (
        typeof maybeResult === "object" &&
        "tool_failed" in maybeResult &&
        maybeResult.tool_failed
      ) {
        return "error";
      }
      return "success";
    }
    if (isToolBusy) return "running";
    // rm tool returns diff message (not tool message) when deleting files with content
    if (hasDiffs) {
      return "success";
    }
    return "running";
  }, [hasDiffs, isToolBusy, maybeResult]);
  const handleFileClick = useCallback(
    (e: React.MouseEvent, filePath: string) => {
      e.stopPropagation();
      void queryPathThenOpenFile({ file_path: filePath });
    },
    [queryPathThenOpenFile],
  );

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const { icon, summary } = useMemo(() => {
    if (toolType === "mv") {
      const mvArgs = args as MvArgs;
      const src = mvArgs.source ?? "";
      const dest = mvArgs.destination ?? "";
      return {
        icon: <MoveIcon />,
        summary: (
          <>
            Move{" "}
            <span
              className={styles.filename}
              onClick={(e) => handleFileClick(e, src)}
            >
              {basename(src)}
            </span>
            {" → "}
            <span
              className={styles.filename}
              onClick={(e) => handleFileClick(e, dest)}
            >
              {basename(dest)}
            </span>
          </>
        ),
      };
    }

    if (toolType === "add_workspace_folder") {
      const addArgs = args as AddWorkspaceArgs;
      const path = addArgs.path ?? "";
      return {
        icon: <PlusCircledIcon />,
        summary: (
          <>
            Add workspace{" "}
            <span className={styles.filename}>{basename(path)}</span>
          </>
        ),
      };
    }

    // rm
    const rmArgs = args as RmArgs;
    const path = rmArgs.path ?? "";
    const isDir = rmArgs.recursive;
    const linesRemoved = allDiffs.reduce(
      (acc, d) => acc + countNonEmptyLines(d.lines_remove),
      0,
    );
    return {
      icon: <TrashIcon />,
      summary: (
        <>
          Delete <span className={styles.filename}>{basename(path)}</span>
          {isDir && <span className={styles.meta}> (recursive)</span>}
          {linesRemoved > 0 && (
            <span className={styles.removed}> −{linesRemoved}</span>
          )}
        </>
      ),
    };
  }, [toolType, args, handleFileClick, allDiffs]);

  return (
    <ToolCard
      icon={icon}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {content && (
        <Box className={styles.resultContent}>
          <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
        </Box>
      )}
    </ToolCard>
  );
};

export default FileOpTool;
