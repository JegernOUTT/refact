import React, { useMemo, useCallback } from "react";
import { FileTextIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./ReadTool.module.css";

interface ReadToolArgs {
  paths?: string;
}

function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

interface ReadToolProps {
  toolCall: ToolCall;
  contextFiles?: ChatContextFile[];
}

export const ReadTool: React.FC<ReadToolProps> = ({
  toolCall,
  contextFiles,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const args = useMemo<ReadToolArgs>(() => {
    try {
      return JSON.parse(toolCall.function.arguments) as ReadToolArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const paths = useMemo(() => {
    return (
      args.paths
        ?.split(",")
        .map((p) => p.trim())
        .filter(Boolean) ?? []
    );
  }, [args.paths]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult]);

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

  const summary = useMemo(() => {
    if (paths.length === 0) return "Read file";
    if (paths.length === 1) {
      return (
        <>
          Read{" "}
          <span
            className={styles.filename}
            onClick={(e) => handleFileClick(e, paths[0])}
          >
            {basename(paths[0])}
          </span>
        </>
      );
    }
    return (
      <>
        Read{" "}
        {paths.map((p, i) => (
          <React.Fragment key={p}>
            {i > 0 && ", "}
            <span
              className={styles.filename}
              onClick={(e) => handleFileClick(e, p)}
            >
              {basename(p)}
            </span>
          </React.Fragment>
        ))}
      </>
    );
  }, [paths, handleFileClick]);

  return (
    <ToolCard
      icon={<FileTextIcon />}
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
      {contextFiles && contextFiles.length > 0 && (
        <ContextFileList files={contextFiles} />
      )}
    </ToolCard>
  );
};

export default ReadTool;
