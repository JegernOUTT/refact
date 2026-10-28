import React, { useMemo } from "react";
import { FileText } from "lucide-react";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../features/Chat/Thread/selectors";
import { useThreadId } from "../../features/Chat/Thread";
import type { ToolCall } from "../../services/refact/types";
import { Markdown } from "../Markdown";
import {
  COMPACT_HEADLINE_CHARS,
  compactFacts,
  compactText,
  readCompactRecord,
} from "../Tools/AgenticTool/compactPresentation";
import { Badge } from "../ui";
import { useUnrecordedResultState } from "./TranscriptItem/useUnrecordedResultState";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptStatus,
} from "./TranscriptItem";
import styles from "./TaskDocumentsView.module.css";

type ToolType = "doc_list" | "doc_get";
type Kind = "plan" | "design" | "runbook" | "brief" | "postmortem" | "spec";
type BadgeTone = React.ComponentProps<typeof Badge>["tone"];
type Row = {
  slug: string;
  name: string;
  kind: string;
  pinned: boolean;
  version: string;
  updated_at: string;
};
type Meta = {
  slug?: string;
  name?: string;
  kind?: string;
  pinned?: string;
  version?: string;
};
type ParsedDocument = {
  body: string;
  meta: Meta;
};
type Props = { toolType: ToolType; content: string };
type TaskDocumentsToolProps = { toolCall: ToolCall; toolType: ToolType };

const KIND_TONES: Record<Kind, BadgeTone> = {
  plan: "accent",
  design: "accent",
  runbook: "success",
  brief: "accent",
  postmortem: "warning",
  spec: "warning",
};

function tableCells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

function parsePinned(value: string): boolean {
  return ["true", "yes", "1", "★", "⭐"].includes(value.trim().toLowerCase());
}

function headerKey(cell: string): keyof Row | null {
  const normalized = cell
    .trim()
    .toLowerCase()
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ");
  switch (normalized) {
    case "slug":
      return "slug";
    case "name":
    case "title":
      return "name";
    case "kind":
      return "kind";
    case "pinned":
      return "pinned";
    case "version":
      return "version";
    case "updated":
    case "updated at":
      return "updated_at";
    default:
      return null;
  }
}

function columnIndex(header: string[], key: keyof Row): number {
  return header.findIndex((cell) => headerKey(cell) === key);
}

function cellValue(cells: string[], index: number, fallback: string): string {
  if (index < 0) return fallback;
  return cells[index] ?? fallback;
}

function kindTone(kind: string): BadgeTone {
  return kind in KIND_TONES ? KIND_TONES[kind as Kind] : "muted";
}

function parseRows(markdown: string): Row[] {
  const lines = markdown
    .split(/\r?\n/)
    .filter((line) => line.trim().startsWith("|"));
  const headerIndex = lines.findIndex((line) => {
    const header = tableCells(line).map(headerKey);
    return (
      header.includes("slug") &&
      header.includes("name") &&
      header.includes("kind")
    );
  });
  if (headerIndex < 0) return [];

  const header = tableCells(lines[headerIndex]);
  const slugIndex = columnIndex(header, "slug");
  const nameIndex = columnIndex(header, "name");
  const kindIndex = columnIndex(header, "kind");
  const pinnedIndex = columnIndex(header, "pinned");
  const versionIndex = columnIndex(header, "version");
  const updatedAtIndex = columnIndex(header, "updated_at");
  return lines.slice(headerIndex + 1).flatMap((line) => {
    const cells = tableCells(line);
    if (cells.every((cell) => /^:?-+:?$/.test(cell))) return [];
    const slug = cellValue(cells, slugIndex, "").trim();
    if (!slug) return [];
    return [
      {
        slug,
        name: cellValue(cells, nameIndex, slug) || slug,
        kind: cellValue(cells, kindIndex, "document") || "document",
        pinned: parsePinned(cellValue(cells, pinnedIndex, "false")),
        version: cellValue(cells, versionIndex, "0") || "0",
        updated_at: cellValue(cells, updatedAtIndex, ""),
      },
    ];
  });
}

function parseDocument(markdown: string): ParsedDocument {
  const lines = markdown.split(/\r?\n/);
  if (lines[0]?.trim() !== "---") return { body: markdown, meta: {} };
  const end = lines.findIndex(
    (line, index) => index > 0 && line.trim() === "---",
  );
  if (end < 0) return { body: markdown, meta: {} };
  const meta: Meta = {};
  for (const line of lines.slice(1, end)) {
    const match = /^(slug|name|kind|pinned|version):\s*(.*)$/.exec(line);
    if (!match) continue;
    const key = match[1] as keyof Meta;
    meta[key] = match[2].trim().replace(/^['"]|['"]$/g, "");
  }
  return {
    body: lines
      .slice(end + 1)
      .join("\n")
      .trim(),
    meta,
  };
}

function hasParsedDocumentContent(document: ParsedDocument): boolean {
  if (document.body.trim().length > 0) return true;
  return Object.values(document.meta).some((value) => value.trim().length > 0);
}

const RawMarkdown: React.FC<{ content: string }> = ({ content }) => (
  <div className={styles.markdown}>
    <Markdown>{content}</Markdown>
  </div>
);

const PinStar: React.FC<{ pinned: boolean; slug?: string }> = ({
  pinned,
  slug,
}) => (
  <span
    aria-label={`${pinned ? "Pinned" : "Not pinned"}${slug ? ` ${slug}` : ""}`}
    className={pinned ? styles.starPinned : styles.star}
  >
    {pinned ? "★" : "☆"}
  </span>
);

export const TaskDocumentsContent: React.FC<Props> = ({
  toolType,
  content,
}) => {
  const rows = useMemo(() => parseRows(content), [content]);
  const document = useMemo(() => parseDocument(content), [content]);

  if (toolType === "doc_get") {
    if (!hasParsedDocumentContent(document))
      return <RawMarkdown content={content} />;
    return <RawMarkdown content={document.body || content} />;
  }

  if (rows.length === 0) return <RawMarkdown content={content} />;

  return (
    <div className={styles.root}>
      <div className={styles.rows}>
        {rows.map((row) => (
          <article key={row.slug} className={styles.row}>
            <div className={styles.rowBody}>
              <div className={styles.identity}>
                <PinStar pinned={row.pinned} slug={row.slug} />
                <div className={styles.identityText}>
                  <span className={styles.name}>{row.name}</span>
                  <span className={styles.slug}>{row.slug}</span>
                </div>
              </div>
              <div className={styles.rowMeta}>
                <Badge tone={kindTone(row.kind)}>{row.kind}</Badge>
                <Badge tone="muted">v{row.version}</Badge>
                <span className={styles.updatedAt}>{row.updated_at}</span>
              </div>
            </div>
          </article>
        ))}
      </div>
    </div>
  );
};

export const TaskDocumentsView: React.FC<TaskDocumentsToolProps> = ({
  toolCall,
  toolType,
}) => {
  const threadId = useThreadId();
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const failed = maybeResult?.tool_failed === true;
  const rows = useMemo(
    () =>
      toolType === "doc_list" && content && !failed ? parseRows(content) : [],
    [toolType, content, failed],
  );
  const document = useMemo(
    () =>
      toolType === "doc_get" && content && !failed
        ? parseDocument(content)
        : null,
    [toolType, content, failed],
  );
  const args = readCompactRecord(toolCall.function.arguments);
  const requestedSlug =
    typeof args?.slug === "string" ? compactText(args.slug) : null;
  const name =
    compactText(document?.meta.name, COMPACT_HEADLINE_CHARS) ??
    compactText(document?.meta.slug);
  const facts =
    toolType === "doc_list"
      ? []
      : compactFacts([
          { label: "Slug", value: document?.meta.slug ?? requestedSlug },
          { label: "Version", value: document?.meta.version },
          { label: "Kind", value: document?.meta.kind },
          {
            label: "Pinned",
            value:
              document?.meta.pinned && parsePinned(document.meta.pinned)
                ? "Yes"
                : null,
          },
        ]);
  const status: TranscriptStatus = !maybeResult
    ? unrecorded.status
    : failed
      ? "failed"
      : "done";
  const target =
    toolType === "doc_list"
      ? rows.length > 0
        ? `${rows.length} document${rows.length === 1 ? "" : "s"}`
        : undefined
      : name ?? requestedSlug ?? undefined;
  const meta =
    toolType === "doc_get" && document?.meta.version
      ? `v${document.meta.version}`
      : undefined;

  return (
    <TranscriptItem
      data-testid="task-documents-view"
      icon={<FileText aria-hidden />}
      verb={toolType === "doc_list" ? "Documents" : "Document"}
      target={target}
      meta={meta}
      status={status}
      statusLabel={!maybeResult ? unrecorded.statusLabel : undefined}
      summary={facts}
      output={
        content !== null && !failed ? (
          <TranscriptClamp>
            <TaskDocumentsContent toolType={toolType} content={content} />
          </TranscriptClamp>
        ) : undefined
      }
      error={failed ? content : undefined}
      raw={[
        { label: "Arguments", value: toolCall.function.arguments },
        ...(content !== null ? [{ label: "Response", value: content }] : []),
      ]}
      openKey={toolCall.id ? `${toolType}:${toolCall.id}` : undefined}
    />
  );
};

export default TaskDocumentsView;
