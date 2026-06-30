import React, { useMemo } from "react";
import { FileText } from "lucide-react";
import { useAppSelector } from "../../hooks";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectToolResultByThreadAndId,
} from "../../features/Chat/Thread/selectors";
import { useThreadId } from "../../features/Chat/Thread";
import type { ToolCall } from "../../services/refact/types";
import { Markdown } from "../Markdown";
import { Badge, Icon } from "../ui";
import { ToolCard, type ToolStatus } from "./ToolCard";
import { useStoredOpen } from "./useStoredOpen";
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

const RawMarkdownFallback: React.FC<{ content: string; notice: string }> = ({
  content,
  notice,
}) => (
  <div className={styles.root}>
    <div className={styles.header}>
      <span className={styles.headerMeta}>{notice}</span>
    </div>
    <div className={styles.markdown}>
      <Markdown>{content}</Markdown>
    </div>
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
    const { body, meta } = document;
    if (!hasParsedDocumentContent(document)) {
      return (
        <RawMarkdownFallback
          content={content}
          notice="Parser produced no document details; raw output below"
        />
      );
    }

    return (
      <div className={styles.root}>
        <div className={styles.header}>
          <span className={styles.headerTitle}>
            {meta.name ?? meta.slug ?? "Task document"}
          </span>
          {meta.version && <Badge tone="muted">v{meta.version}</Badge>}
        </div>
        <div className={styles.badges}>
          {meta.slug && <Badge tone="muted">{meta.slug}</Badge>}
          {meta.kind && <Badge tone={kindTone(meta.kind)}>{meta.kind}</Badge>}
          {meta.pinned && <PinStar pinned={parsePinned(meta.pinned)} />}
        </div>
        <div className={styles.markdown}>
          <Markdown>{body || content}</Markdown>
        </div>
      </div>
    );
  }

  if (rows.length === 0) {
    return (
      <RawMarkdownFallback
        content={content}
        notice="Parser produced no rows; raw output below"
      />
    );
  }

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <span className={styles.headerTitle}>Task documents</span>
        <span className={styles.headerMeta}>{rows.length} documents</span>
      </div>
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
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const threadId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, threadId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, threadId),
  );
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const rows = useMemo(() => (content ? parseRows(content) : []), [content]);
  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    return maybeResult.tool_failed ? "error" : "success";
  }, [isStreaming, isWaiting, maybeResult]);

  return (
    <>
      <span data-testid="task-documents-view" hidden />
      <ToolCard
        icon={<Icon icon={FileText} size="sm" />}
        summary={toolType === "doc_list" ? "Task documents" : "Task document"}
        meta={
          toolType === "doc_list" && content && rows.length > 0
            ? `${rows.length} documents`
            : undefined
        }
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        {content && (
          <TaskDocumentsContent toolType={toolType} content={content} />
        )}
      </ToolCard>
    </>
  );
};

export default TaskDocumentsView;
