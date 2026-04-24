import React, { useMemo } from "react";
import { GlobeIcon, MagnifyingGlassIcon } from "@radix-ui/react-icons";
import { Box, Flex, Text } from "@radix-ui/themes";

import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { Link } from "../../Link";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import styles from "./WebTool.module.css";

type WebToolType = "web" | "web_search";

interface WebArgs {
  url?: string;
}

interface WebSearchArgs {
  query?: string;
  num_results?: number | string;
}

type SearchResult = {
  title: string;
  url: string;
  snippet: string;
};

interface WebToolProps {
  toolCall: ToolCall;
  toolType: WebToolType;
  contextFiles?: ChatContextFile[];
}

function extractDomain(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.hostname;
  } catch {
    return url;
  }
}

function isSafeHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
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
  return hasTableHeader && hasTableSep;
}

function parseSearchResultLine(line: string): SearchResult | null {
  const trimmed = line.trim();
  if (!trimmed) return null;

  const match = trimmed.match(/^\d+\.\s+\[(.+?)\]\((.+?)\)$/);
  if (!match) return null;

  const [, rawTitle, rawUrl] = match;
  const title = rawTitle.trim();
  const url = rawUrl.trim();
  if (!title || !url) return null;

  return {
    title,
    url,
    snippet: "",
  };
}

function parseSearchResultsFromText(content: string): SearchResult[] {
  const lines = content.split("\n");
  const results: SearchResult[] = [];
  let current: SearchResult | null = null;

  for (const line of lines) {
    const parsedResult = parseSearchResultLine(line);
    if (parsedResult) {
      current = parsedResult;
      results.push(current);
      continue;
    }

    if (!current) continue;
    if (line.startsWith("   ")) {
      const snippet = line.trim();
      if (snippet) {
        current.snippet = current.snippet
          ? `${current.snippet} ${snippet}`
          : snippet;
      }
    } else if (line.trim() === "") {
      current = null;
    }
  }

  return results;
}

function isSearchResult(value: unknown): value is SearchResult {
  return (
    typeof value === "object" &&
    value !== null &&
    "title" in value &&
    typeof value.title === "string" &&
    "url" in value &&
    typeof value.url === "string" &&
    "snippet" in value &&
    typeof value.snippet === "string"
  );
}

function extractStructuredSearchResults(
  extra: Record<string, unknown> | undefined,
): SearchResult[] {
  const raw = extra?.search_results;
  if (!Array.isArray(raw)) return [];

  return raw.filter(isSearchResult).slice(0, 50);
}

export const WebTool: React.FC<WebToolProps> = ({
  toolCall,
  toolType,
  contextFiles,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const args = useMemo((): WebArgs | WebSearchArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as WebArgs | WebSearchArgs;
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

  const searchResults = useMemo(() => {
    if (toolType !== "web_search") return [] as SearchResult[];

    const structured = extractStructuredSearchResults(maybeResult?.extra);
    if (structured.length > 0) return structured;
    if (!content) return [] as SearchResult[];
    return parseSearchResultsFromText(content).slice(0, 50);
  }, [toolType, content, maybeResult?.extra]);

  const summary = useMemo(() => {
    if (toolType === "web") {
      const webArgs = args as WebArgs;
      const url = webArgs.url ?? "page";
      return (
        <>
          Fetch <span className={styles.url}>{extractDomain(url)}</span>
        </>
      );
    }

    const searchArgs = args as WebSearchArgs;
    const query = searchArgs.query ?? "query";
    return (
      <>
        Search web <span className={styles.query}>&quot;{query}&quot;</span>
      </>
    );
  }, [toolType, args]);

  const meta = useMemo(() => {
    if (toolType !== "web_search") return undefined;

    if (searchResults.length > 0) {
      return `${searchResults.length} result${
        searchResults.length === 1 ? "" : "s"
      }`;
    }

    const requested = (args as WebSearchArgs).num_results;
    if (typeof requested === "number") return `up to ${requested}`;
    if (typeof requested === "string" && requested.trim()) {
      return `up to ${requested.trim()}`;
    }

    return undefined;
  }, [toolType, searchResults.length, args]);

  const shouldRenderMarkdown =
    !!content && content.length <= 50_000 && looksLikeMarkdown(content);

  return (
    <ToolCard
      icon={toolType === "web_search" ? <MagnifyingGlassIcon /> : <GlobeIcon />}
      summary={summary}
      meta={meta}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {toolType === "web_search" && searchResults.length > 0 && (
        <Box>
          <Text size="1" color="gray">
            Results ({searchResults.length})
          </Text>
          <Box className={styles.resultList}>
            {searchResults.map((result, idx) => {
              const safeUrl = isSafeHttpUrl(result.url) ? result.url : "";
              return (
                <Box key={`${result.url}-${idx}`} className={styles.resultItem}>
                  <Flex direction="column" gap="1">
                    {safeUrl ? (
                      <Link
                        href={safeUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        size="2"
                      >
                        {result.title}
                      </Link>
                    ) : (
                      <Text size="2" weight="medium">
                        {result.title}
                      </Text>
                    )}

                    {safeUrl && (
                      <Text size="1" color="gray" className={styles.inlineCode}>
                        {safeUrl}
                      </Text>
                    )}

                    {result.snippet && (
                      <Text size="1" color="gray">
                        {result.snippet}
                      </Text>
                    )}
                  </Flex>
                </Box>
              );
            })}
          </Box>
        </Box>
      )}

      {content && !(toolType === "web_search" && searchResults.length > 0) && (
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

      {contextFiles && contextFiles.length > 0 && (
        <ContextFileList files={contextFiles} />
      )}
    </ToolCard>
  );
};

export default WebTool;
