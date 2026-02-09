import React, { useMemo, useState, useCallback } from "react";
import { MagnifyingGlassIcon } from "@radix-ui/react-icons";
import { Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import styles from "./SearchTool.module.css";

type SearchToolType =
  | "search_pattern"
  | "search_semantic"
  | "search_symbol_definition";

interface SearchPatternArgs {
  pattern?: string;
  scope?: string;
}

interface SearchSemanticArgs {
  queries?: string;
  scope?: string;
}

interface SearchSymbolArgs {
  symbols?: string;
}

interface SearchToolProps {
  toolCall: ToolCall;
  toolType: SearchToolType;
  contextFiles?: ChatContextFile[];
}

function truncate(str: string, maxLen: number): string {
  if (str.length <= maxLen) return str;
  return str.slice(0, maxLen - 1) + "…";
}

function countMatches(content: string): number | null {
  const lines = content.split("\n").filter((l) => l.trim());
  if (lines.length === 0) return null;
  return lines.length;
}

export const SearchTool: React.FC<SearchToolProps> = ({
  toolCall,
  toolType,
  contextFiles,
}) => {
  const [isOpen, setIsOpen] = useState(false);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const args = useMemo(():
    | SearchPatternArgs
    | SearchSemanticArgs
    | SearchSymbolArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as
        | SearchPatternArgs
        | SearchSemanticArgs
        | SearchSymbolArgs;
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

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  // Don't show match count on error - error messages also have content
  const matchCount =
    content && status !== "error" ? countMatches(content) : null;

  const summary = useMemo(() => {
    switch (toolType) {
      case "search_pattern": {
        const patternArgs = args as SearchPatternArgs;
        const pattern = patternArgs.pattern ?? "pattern";
        return (
          <>
            Search <span className={styles.query}>{truncate(pattern, 30)}</span>
            {matchCount !== null && (
              <span className={styles.count}> → {matchCount} matches</span>
            )}
          </>
        );
      }
      case "search_semantic": {
        const semanticArgs = args as SearchSemanticArgs;
        const query = semanticArgs.queries ?? "query";
        return (
          <>
            Search{" "}
            <span className={styles.query}>
              &quot;{truncate(query, 25)}&quot;
            </span>
            {matchCount !== null && (
              <span className={styles.count}> → {matchCount} results</span>
            )}
          </>
        );
      }
      case "search_symbol_definition": {
        const symbolArgs = args as SearchSymbolArgs;
        const symbols = symbolArgs.symbols ?? "symbol";
        return (
          <>
            Find <span className={styles.query}>{truncate(symbols, 30)}</span>
            {matchCount !== null && (
              <span className={styles.count}> → {matchCount} found</span>
            )}
          </>
        );
      }
    }
  }, [toolType, args, matchCount]);

  const meta = useMemo(() => {
    if (toolType === "search_pattern" || toolType === "search_semantic") {
      const scopeArgs = args as SearchPatternArgs | SearchSemanticArgs;
      if (scopeArgs.scope && scopeArgs.scope !== "workspace") {
        return scopeArgs.scope;
      }
    }
    return null;
  }, [toolType, args]);

  return (
    <ToolCard
      icon={<MagnifyingGlassIcon />}
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

export default SearchTool;
