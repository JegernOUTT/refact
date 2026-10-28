import { Plug, Settings, Tag } from "lucide-react";
import React, { useMemo } from "react";
import type { ToolStatus } from "./ToolCard";
import { ToolCallTooltip } from "./ToolCallTooltip";
import { useToolCallActivity } from "./useToolCallActivity";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import {
  getToolEnrichment,
  type ToolCall,
  type ToolEnrichmentReference,
} from "../../../services/refact/types";
import { Markdown, ShikiCodeBlock } from "../../Markdown";
import {
  TranscriptClamp,
  TranscriptFacts,
  TranscriptItem,
  fromToolTone,
  type TranscriptFact,
  type TranscriptRawBlock,
} from "../TranscriptItem";
import { describeToolCall, parseToolArgs } from "./genericToolTitle";
import styles from "./GenericTool.module.css";

interface GenericToolProps {
  toolCall: ToolCall;
}

const RESULT_HEAD_CHARS = 30_000;
const RESULT_TAIL_CHARS = 10_000;
const MARKDOWN_MAX_CHARS = 50_000;
const FACT_MAX_CHARS = 200;
const CODE_ARG_KEYS = new Set([
  "code",
  "script",
  "command",
  "cmd",
  "sql",
  "function",
  "expression",
  "source",
  "patch",
  "diff",
  "content",
]);

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

function capToolResult(text: string): string {
  const maxChars = RESULT_HEAD_CHARS + RESULT_TAIL_CHARS;
  if (text.length <= maxChars) return text;
  const hidden = text.length - maxChars;
  return `${text.slice(
    0,
    RESULT_HEAD_CHARS,
  )}\n… ${hidden.toLocaleString()} chars hidden, see Raw …\n${text.slice(
    -RESULT_TAIL_CHARS,
  )}`;
}

function isCodeArgument(key: string, value: unknown): value is string {
  if (typeof value !== "string") return false;
  return CODE_ARG_KEYS.has(key) || value.includes("\n");
}

function factValue(value: unknown): string {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > FACT_MAX_CHARS
    ? `${text.slice(0, FACT_MAX_CHARS - 1)}…`
    : text;
}

export function ArgumentsView({ args }: { args: Record<string, unknown> }) {
  const entries = Object.entries(args);
  const codeEntries = entries.filter(([key, value]) =>
    isCodeArgument(key, value),
  );
  const facts: TranscriptFact[] = entries
    .filter(([key, value]) => !isCodeArgument(key, value))
    .map(([key, value]) => {
      const text = factValue(value);
      return { label: key, value: text, title: text };
    });
  return (
    <div className={styles.argumentList}>
      {facts.length > 0 ? <TranscriptFacts facts={facts} /> : null}
      {codeEntries.map(([key, value]) => (
        <div className={styles.argumentCode} key={key}>
          <span className={styles.argumentLabel}>{key}</span>
          <ShikiCodeBlock showLineNumbers={false}>
            {String(value)}
          </ShikiCodeBlock>
        </div>
      ))}
    </div>
  );
}

export function ResultView({ content }: { content: string }) {
  const rendered = useMemo(() => capToolResult(content), [content]);
  const asMarkdown =
    content.length <= MARKDOWN_MAX_CHARS && looksLikeMarkdown(content);
  return (
    <TranscriptClamp lineCount={content.split("\n").length}>
      {asMarkdown ? (
        <div className={styles.markdownContent}>
          <Markdown>{rendered}</Markdown>
        </div>
      ) : (
        <ShikiCodeBlock showLineNumbers={false}>{rendered}</ShikiCodeBlock>
      )}
    </TranscriptClamp>
  );
}

function formatReference(reference: ToolEnrichmentReference): string {
  const range = reference.line1
    ? `:${reference.line1}-${reference.line2 ?? reference.line1}`
    : "";
  const count = reference.count ? ` (${reference.count})` : "";
  const state = reference.status ? ` (${reference.status})` : "";
  return `${reference.kind}: ${
    reference.label ?? reference.target
  }${range}${count}${state}`;
}

function toolIcon(name: string) {
  if (name === "set_trajectory_label") return <Tag />;
  if (name.startsWith("mcp_") || name.includes("_")) return <Plug />;
  return <Settings />;
}

export const GenericTool: React.FC<GenericToolProps> = ({ toolCall }) => {
  const { result } = useToolCallActivity(toolCall.id);
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const toolName = toolCall.function.name ?? "tool";
  const rawArguments = toolCall.function.arguments;
  const args = useMemo(() => parseToolArgs(rawArguments), [rawArguments]);
  const title = useMemo(
    () => describeToolCall(toolName, rawArguments),
    [toolName, rawArguments],
  );

  const tone: ToolStatus = !result
    ? unrecorded.status === "running"
      ? "running"
      : "idle"
    : result.tool_failed
      ? "error"
      : "success";
  const status = result ? fromToolTone(tone) : unrecorded.status;
  const content =
    result && typeof result.content === "string" ? result.content : null;
  const enrichment = getToolEnrichment(result?.extra);
  const references =
    enrichment && !enrichment.privacy?.restricted ? enrichment.references : [];

  const summary: TranscriptFact[] = references.length
    ? [
        {
          label: "References",
          value: (
            <span className={styles.references} data-testid="tool-enrichment">
              {references.map((reference) => (
                <span key={`${reference.kind}:${reference.target}`}>
                  {formatReference(reference)}
                </span>
              ))}
            </span>
          ),
        },
      ]
    : [];

  const raw: TranscriptRawBlock[] = [
    { label: "Arguments", value: rawArguments },
    ...(result ? [{ label: "Response", value: result.content }] : []),
  ];

  return (
    <ToolCallTooltip toolCall={toolCall}>
      <TranscriptItem
        className="rf-enter"
        data-status={tone}
        data-testid="generic-tool"
        icon={toolIcon(toolName)}
        input={
          Object.keys(args).length > 0 ? <ArgumentsView args={args} /> : null
        }
        meta={title.meta}
        openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
        output={content ? <ResultView content={content} /> : null}
        raw={raw}
        status={status}
        statusLabel={result ? undefined : unrecorded.statusLabel}
        summary={summary}
        target={title.target}
        verb={title.verb}
      />
    </ToolCallTooltip>
  );
};

export default GenericTool;
