import { BookOpen, MessageCircle, Pencil } from "lucide-react";
import React, { useMemo } from "react";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./KnowledgeTool.module.css";

type KnowledgeToolType =
  | "knowledge"
  | "create_knowledge"
  | "trajectories"
  | "search_trajectories";

interface KnowledgeArgs {
  search_key?: string;
}

interface CreateKnowledgeArgs {
  content?: string;
}

interface TrajectoriesArgs {
  query?: string;
}

interface KnowledgeToolProps {
  toolCall: ToolCall;
  toolType: KnowledgeToolType;
  contextFiles?: ChatContextFile[];
}

const MAX_TITLE_PREVIEW_CHARS = 80;

function titlePreview(text: string): string {
  const firstLine = text.split("\n", 1)[0].trim();
  if (firstLine.length <= MAX_TITLE_PREVIEW_CHARS) return firstLine;
  return `${firstLine.slice(0, MAX_TITLE_PREVIEW_CHARS)}…`;
}

export const KnowledgeTool: React.FC<KnowledgeToolProps> = ({
  toolCall,
  toolType,
  contextFiles,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const args = useMemo(():
    | KnowledgeArgs
    | CreateKnowledgeArgs
    | TrajectoriesArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as
        | KnowledgeArgs
        | CreateKnowledgeArgs
        | TrajectoriesArgs;
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

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const summary = useMemo(() => {
    if (toolType === "knowledge") {
      const knowledgeArgs = args as KnowledgeArgs;
      const query = knowledgeArgs.search_key ?? "knowledge";
      return (
        <>
          Recall <span className={styles.query}>&quot;{query}&quot;</span>
        </>
      );
    }

    if (toolType === "create_knowledge") {
      const createArgs = args as CreateKnowledgeArgs;
      const preview = titlePreview(createArgs.content ?? "memory");
      return (
        <>
          Remember <span className={styles.query}>&quot;{preview}&quot;</span>
        </>
      );
    }

    // toolType === "trajectories" || toolType === "search_trajectories"
    const trajArgs = args as TrajectoriesArgs;
    const query = trajArgs.query ?? "conversations";
    return (
      <>
        Recall <span className={styles.query}>&quot;{query}&quot;</span>
      </>
    );
  }, [toolType, args]);

  const icon =
    toolType === "create_knowledge" ? (
      <Pencil />
    ) : toolType === "knowledge" ? (
      <BookOpen />
    ) : (
      <MessageCircle />
    );

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
      {contextFiles && contextFiles.length > 0 && (
        <ContextFileList files={contextFiles} />
      )}
    </ToolCard>
  );
};

export default KnowledgeTool;
