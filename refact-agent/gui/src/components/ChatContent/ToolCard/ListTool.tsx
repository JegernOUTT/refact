import { Archive } from "lucide-react";
import React, { useMemo, useCallback } from "react";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./ListTool.module.css";

interface ListToolArgs {
  path?: string;
  use_ast?: boolean;
  max_files?: number;
}

interface ListToolProps {
  toolCall: ToolCall;
  contextFiles?: ChatContextFile[];
}

export const ListTool: React.FC<ListToolProps> = ({
  toolCall,
  contextFiles,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const args = useMemo<ListToolArgs>(() => {
    try {
      return JSON.parse(toolCall.function.arguments) as ListToolArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

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

  const handlePathClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (args.path) {
        void queryPathThenOpenFile({ file_path: args.path });
      }
    },
    [queryPathThenOpenFile, args.path],
  );

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const summary = useMemo(() => {
    const path = args.path ?? "project";
    return (
      <>
        List{" "}
        <span className={styles.path} onClick={handlePathClick}>
          {path}
        </span>
      </>
    );
  }, [args.path, handlePathClick]);

  const meta = useMemo(() => {
    const parts: string[] = [];
    if (args.use_ast) parts.push("AST");
    if (args.max_files) parts.push(`max ${args.max_files}`);
    return parts.length > 0 ? parts.join(" · ") : null;
  }, [args.use_ast, args.max_files]);

  return (
    <ToolCard
      icon={<Archive />}
      summary={summary}
      meta={meta}
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

export default ListTool;
