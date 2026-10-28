import { FolderSearch, FolderTree, Search } from "lucide-react";
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
import styles from "./SearchTool.module.css";

export type SearchToolType =
  | "search_pattern"
  | "search_semantic"
  | "search_symbol_definition"
  | "glob"
  | "tree";

interface SearchArgs {
  pattern?: unknown;
  queries?: unknown;
  symbols?: unknown;
  scope?: unknown;
  path?: unknown;
  use_ast?: unknown;
  max_files?: unknown;
}

interface SearchToolProps {
  toolCall: ToolCall;
  toolType: SearchToolType;
  contextFiles?: ChatContextFile[];
}

type SearchPresentation = {
  icon: React.ReactNode;
  verb: string;
  target: string;
  scope: string | null;
  unit: string;
};

function text(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function parseArgs(raw: string): SearchArgs {
  try {
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object" ? (parsed as SearchArgs) : {};
  } catch {
    return {};
  }
}

function presentation(
  type: SearchToolType,
  args: SearchArgs,
): SearchPresentation {
  const scope = text(args.scope) ?? text(args.path);
  const visibleScope = scope && scope !== "workspace" ? scope : null;
  switch (type) {
    case "search_pattern":
      return {
        icon: <Search />,
        verb: "Search",
        target: text(args.pattern) ?? "pattern",
        scope: visibleScope,
        unit: "matches",
      };
    case "search_semantic":
      return {
        icon: <Search />,
        verb: "Search",
        target: text(args.queries) ?? "query",
        scope: visibleScope,
        unit: "results",
      };
    case "search_symbol_definition":
      return {
        icon: <Search />,
        verb: "Find symbol",
        target: text(args.symbols) ?? "symbol",
        scope: null,
        unit: "definitions",
      };
    case "glob":
      return {
        icon: <FolderSearch />,
        verb: "Find files",
        target: text(args.pattern) ?? "pattern",
        scope: visibleScope,
        unit: "files",
      };
    case "tree":
      return {
        icon: <FolderTree />,
        verb: "Tree",
        target: scope ?? "project",
        scope: null,
        unit: "lines",
      };
  }
}

function countLines(content: string): number {
  return content.split("\n").filter((line) => line.trim().length > 0).length;
}

export const SearchTool: React.FC<SearchToolProps> = ({
  toolCall,
  toolType,
  contextFiles = [],
}) => {
  const threadId = useThreadId();
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const args = useMemo(
    () => parseArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const view = presentation(toolType, args);

  const unrecorded = useUnrecordedResultState(toolCall.id);
  const failed = result?.tool_failed === true;
  const unrecordedRow = result === undefined && contextFiles.length === 0;
  const status: TranscriptStatus = failed
    ? "failed"
    : unrecordedRow
      ? unrecorded.status
      : "done";
  const content = typeof result?.content === "string" ? result.content : null;
  const matchCount = content && !failed ? countLines(content) : 0;
  const fileCount = new Set(contextFiles.map((file) => file.file_name)).size;

  const meta = [
    matchCount > 0 ? `${matchCount} ${view.unit}` : null,
    fileCount > 0 && toolType !== "glob"
      ? `in ${fileCount} file${fileCount === 1 ? "" : "s"}`
      : null,
    toolType === "tree" && args.use_ast === true ? "AST" : null,
  ]
    .filter(Boolean)
    .join(" ");

  const output =
    content && !failed ? (
      <div className={styles.results}>
        <TranscriptClamp lineCount={matchCount}>
          <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
        </TranscriptClamp>
        <ContextFileList files={contextFiles} />
      </div>
    ) : contextFiles.length > 0 ? (
      <ContextFileList files={contextFiles} />
    ) : null;

  return (
    <TranscriptItem
      data-testid="search-tool"
      error={failed ? content : null}
      icon={view.icon}
      input={
        view.scope ? <code className={styles.scope}>{view.scope}</code> : null
      }
      meta={meta}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      sectionLabels={{ input: "Scope" }}
      status={status}
      statusLabel={unrecordedRow ? unrecorded.statusLabel : undefined}
      target={view.target}
      verb={view.verb}
    />
  );
};

export default SearchTool;
