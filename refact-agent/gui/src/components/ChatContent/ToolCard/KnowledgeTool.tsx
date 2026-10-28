import { BookOpen, MessageCircle, Pencil } from "lucide-react";
import React, { useMemo } from "react";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptStatus,
} from "../TranscriptItem";
import styles from "./KnowledgeTool.module.css";

type KnowledgeToolType =
  | "knowledge"
  | "create_knowledge"
  | "trajectories"
  | "search_trajectories";

interface KnowledgeArgs {
  search_key?: string;
  content?: string;
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
  contextFiles = [],
}) => {
  const threadId = useThreadId();
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const args = useMemo((): KnowledgeArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as KnowledgeArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const unrecorded = useUnrecordedResultState(toolCall.id);
  const failed = result?.tool_failed === true;
  const status: TranscriptStatus = !result
    ? unrecorded.status
    : failed
      ? "failed"
      : "done";
  const content = typeof result?.content === "string" ? result.content : null;

  const isSave = toolType === "create_knowledge";
  const target = isSave
    ? titlePreview(args.content ?? "memory")
    : toolType === "knowledge"
      ? args.search_key ?? "knowledge"
      : args.query ?? "conversations";
  const meta =
    contextFiles.length > 0
      ? `${contextFiles.length} result${contextFiles.length === 1 ? "" : "s"}`
      : isSave && result && !failed
        ? "saved"
        : undefined;

  const icon = isSave ? (
    <Pencil />
  ) : toolType === "knowledge" ? (
    <BookOpen />
  ) : (
    <MessageCircle />
  );

  const hasContent = content !== null && content.length > 0 && !failed;
  const output =
    hasContent || contextFiles.length > 0 ? (
      <div className={styles.output}>
        {hasContent && (
          <TranscriptClamp>
            <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
          </TranscriptClamp>
        )}
        <ContextFileList files={contextFiles} />
      </div>
    ) : null;

  return (
    <TranscriptItem
      data-testid="knowledge-tool"
      error={failed ? content : null}
      icon={icon}
      meta={meta}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      status={status}
      statusLabel={!result ? unrecorded.statusLabel : undefined}
      target={target}
      verb="Memory"
    />
  );
};

export default KnowledgeTool;
