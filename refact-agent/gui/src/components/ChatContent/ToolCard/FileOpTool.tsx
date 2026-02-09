import React, { useMemo, useState, useCallback } from "react";
import { MoveIcon, TrashIcon, PlusCircledIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import {
  selectToolResultById,
  selectManyDiffMessageByIds,
} from "../../../features/Chat/Thread/selectors";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { truncateMiddle, basename } from "./utils";
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
}

export const FileOpTool: React.FC<FileOpToolProps> = ({
  toolCall,
  toolType,
  diffs = [],
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const toolDiffs = useAppSelector(
    selectManyDiffMessageByIds(toolCall.id ? [toolCall.id] : []),
  );

  const allDiffs = useMemo((): DiffChunk[] => {
    const fromProps = diffs;
    const fromStore = toolDiffs.flatMap((d) => d.content);
    return fromProps.length > 0 ? fromProps : fromStore;
  }, [diffs, toolDiffs]);

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
    // rm tool returns diff message (not tool message) when deleting files with content
    if (toolDiffs.length > 0) {
      return "success";
    }
    return "running";
  }, [maybeResult, toolDiffs]);

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

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
              {truncateMiddle(basename(src), 20)}
            </span>
            {" → "}
            <span
              className={styles.filename}
              onClick={(e) => handleFileClick(e, dest)}
            >
              {truncateMiddle(basename(dest), 20)}
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
            <span className={styles.filename}>
              {truncateMiddle(basename(path), 30)}
            </span>
          </>
        ),
      };
    }

    // rm
    const rmArgs = args as RmArgs;
    const path = rmArgs.path ?? "";
    const isDir = rmArgs.recursive;
    const linesRemoved = allDiffs.reduce((acc, d) => {
      return (
        acc + d.lines_remove.split("\n").filter((l) => l.length > 0).length
      );
    }, 0);
    return {
      icon: <TrashIcon />,
      summary: (
        <>
          Delete{" "}
          <span className={styles.filename}>
            {truncateMiddle(basename(path), 30)}
          </span>
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
