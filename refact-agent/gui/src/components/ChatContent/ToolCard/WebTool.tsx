import { Globe, Search } from "lucide-react";
import React, { useMemo } from "react";

import { ContextFileList } from "./ContextFileList";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
import { Link } from "../../Link";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptStatus,
} from "../TranscriptItem";
import styles from "./WebTool.module.css";

type WebToolType = "web" | "web_search";

interface WebArgs {
  url?: string;
  query?: string;
  num_results?: number | string;
}

const MAX_RESULTS = 50;
const MAX_MARKDOWN_CHARS = 50_000;

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

  return raw.filter(isSearchResult).slice(0, MAX_RESULTS);
}

export const WebTool: React.FC<WebToolProps> = ({
  toolCall,
  toolType,
  contextFiles = [],
}) => {
  const threadId = useThreadId();
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );

  const args = useMemo((): WebArgs => {
    try {
      return JSON.parse(toolCall.function.arguments) as WebArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const failed = result?.tool_failed === true;
  const status: TranscriptStatus = !result
    ? "running"
    : failed
      ? "failed"
      : "done";
  const content = typeof result?.content === "string" ? result.content : null;

  const searchResults = useMemo(() => {
    if (toolType !== "web_search") return [];
    const structured = extractStructuredSearchResults(result?.extra);
    if (structured.length > 0) return structured;
    if (!content) return [];
    return parseSearchResultsFromText(content).slice(0, MAX_RESULTS);
  }, [toolType, content, result?.extra]);

  const isSearch = toolType === "web_search";
  const target = isSearch
    ? args.query ?? "query"
    : extractDomain(args.url ?? "page");

  const requested =
    typeof args.num_results === "number"
      ? String(args.num_results)
      : args.num_results?.trim();
  const meta = !isSearch
    ? undefined
    : searchResults.length > 0
      ? `${searchResults.length} result${searchResults.length === 1 ? "" : "s"}`
      : requested
        ? `up to ${requested}`
        : undefined;

  const shouldRenderMarkdown =
    !!content &&
    content.length <= MAX_MARKDOWN_CHARS &&
    looksLikeMarkdown(content);

  const results =
    searchResults.length > 0 ? (
      <ul className={styles.resultList}>
        {searchResults.map((item, idx) => {
          const safeUrl = isSafeHttpUrl(item.url) ? item.url : "";
          return (
            <li className={styles.resultItem} key={`${item.url}-${idx}`}>
              {safeUrl ? (
                <Link href={safeUrl} rel="noopener noreferrer" target="_blank">
                  {item.title}
                </Link>
              ) : (
                <span>{item.title}</span>
              )}
              {safeUrl && <span className={styles.inlineCode}>{safeUrl}</span>}
              {item.snippet && (
                <span className={styles.snippet}>{item.snippet}</span>
              )}
            </li>
          );
        })}
      </ul>
    ) : content && !failed ? (
      shouldRenderMarkdown ? (
        <div className={styles.markdownContent}>
          <Markdown>{content}</Markdown>
        </div>
      ) : (
        <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
      )
    ) : null;

  const output =
    results !== null || contextFiles.length > 0 ? (
      <div className={styles.output}>
        {results && <TranscriptClamp>{results}</TranscriptClamp>}
        <ContextFileList files={contextFiles} />
      </div>
    ) : null;

  return (
    <TranscriptItem
      data-testid="web-tool"
      error={failed ? content : null}
      icon={isSearch ? <Search /> : <Globe />}
      input={
        !isSearch && args.url ? (
          <code className={styles.inlineCode}>{args.url}</code>
        ) : null
      }
      meta={meta}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      sectionLabels={{ input: "URL" }}
      status={status}
      target={target}
      verb="Web"
    />
  );
};

export default WebTool;
