import { ExternalLink, FileText } from "lucide-react";
import React, { useMemo } from "react";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { Button } from "../../ui";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptStatus,
} from "../TranscriptItem";
import {
  contextFileLineCount,
  filesTarget,
  normalizeReadPaths,
  type ReadToolArgs,
} from "./readToolPaths";
import styles from "./ReadTool.module.css";

const PATHS_FOUND_PREFIX = "Paths found:";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

interface ReadToolProps {
  toolCall: ToolCall;
  contextFiles?: ChatContextFile[];
}

export const ReadTool: React.FC<ReadToolProps> = ({
  toolCall,
  contextFiles = [],
}) => {
  const { canOpen, openFile } = useOpenFileInApp();
  const threadId = useThreadId();
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const paths = useMemo(() => {
    try {
      const parsed = JSON.parse(toolCall.function.arguments) as unknown;
      return normalizeReadPaths(
        isRecord(parsed) ? (parsed as ReadToolArgs) : {},
      );
    } catch {
      return [];
    }
  }, [toolCall.function.arguments]);

  const failed = result?.tool_failed === true;
  const status: TranscriptStatus = !result
    ? contextFiles.length > 0
      ? "done"
      : "running"
    : failed
      ? "failed"
      : "done";
  const content = typeof result?.content === "string" ? result.content : null;
  const plainContent =
    content && !failed && !content.trimStart().startsWith(PATHS_FOUND_PREFIX)
      ? content
      : null;

  const fileCount = contextFiles.length || paths.length;
  const lineCount = contextFiles.reduce(
    (sum, file) => sum + contextFileLineCount(file),
    0,
  );
  const meta = [
    fileCount > 1 ? `${fileCount} files` : null,
    lineCount > 0 ? `${lineCount} lines` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  const output =
    contextFiles.length > 0 ? (
      <ContextFileList files={contextFiles} />
    ) : plainContent ? (
      <TranscriptClamp>
        <ShikiCodeBlock showLineNumbers={false}>{plainContent}</ShikiCodeBlock>
      </TranscriptClamp>
    ) : null;

  const openTargets =
    paths.length > 0 ? paths : contextFiles.map((f) => f.file_name);
  const artifacts =
    canOpen && openTargets.length > 0 ? (
      <div className={styles.artifacts}>
        {openTargets.map((path) => (
          <Button
            key={path}
            leftIcon={ExternalLink}
            size="sm"
            title={path}
            variant="soft"
            onClick={() => openFile({ path })}
          >
            {path.split("/").pop() ?? path}
          </Button>
        ))}
      </div>
    ) : null;

  return (
    <TranscriptItem
      artifacts={artifacts}
      data-testid="read-tool"
      error={failed ? content : null}
      icon={<FileText />}
      meta={meta}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      sectionLabels={{ artifacts: "Open in IDE" }}
      status={status}
      target={filesTarget(paths, "file")}
      verb="Read"
    />
  );
};

export default ReadTool;
