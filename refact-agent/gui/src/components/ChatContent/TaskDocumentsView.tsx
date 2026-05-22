import React, { useMemo } from "react";
import { Badge, Box, Flex, Text } from "@radix-ui/themes";
import { FileTextIcon } from "@radix-ui/react-icons";
import { useAppSelector } from "../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectToolResultById,
} from "../../features/Chat/Thread/selectors";
import type { ToolCall } from "../../services/refact/types";
import { Markdown } from "../Markdown";
import { ToolCard, type ToolStatus } from "./ToolCard";
import { useStoredOpen } from "./useStoredOpen";
import styles from "./TaskDocumentsView.module.css";

type ToolType = "doc_list" | "doc_get";
type Kind = "plan" | "design" | "runbook" | "brief" | "postmortem" | "spec";
type BadgeColor = React.ComponentProps<typeof Badge>["color"];
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

const KIND_COLORS: Record<Kind, BadgeColor> = {
  plan: "blue",
  design: "purple",
  runbook: "green",
  brief: "cyan",
  postmortem: "amber",
  spec: "orange",
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

function kindColor(kind: string): BadgeColor {
  return kind in KIND_COLORS ? KIND_COLORS[kind as Kind] : "gray";
}

function parseRows(markdown: string): Row[] {
  const lines = markdown
    .split(/\r?\n/)
    .filter((line) => line.trim().startsWith("|"));
  const headerIndex = lines.findIndex((line) => {
    const header = tableCells(line).map((cell) => cell.toLowerCase());
    return (
      header.includes("slug") &&
      header.includes("name") &&
      header.includes("kind")
    );
  });
  if (headerIndex < 0) return [];

  const header = tableCells(lines[headerIndex]).map((cell) =>
    cell.toLowerCase(),
  );
  const index = (name: keyof Row) => header.indexOf(name);
  return lines.slice(headerIndex + 1).flatMap((line) => {
    const cells = tableCells(line);
    if (cells.every((cell) => /^:?-+:?$/.test(cell))) return [];
    const slug = cells[index("slug")] ?? "";
    if (!slug) return [];
    return [
      {
        slug,
        name: cells[index("name")] ?? slug,
        kind: cells[index("kind")] ?? "document",
        pinned: parsePinned(cells[index("pinned")] ?? "false"),
        version: cells[index("version")] ?? "0",
        updated_at: cells[index("updated_at")] ?? "",
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
  <Box className={styles.root}>
    <Box className={styles.header}>
      <Text size="1" color="gray">
        {notice}
      </Text>
    </Box>
    <Box className={styles.markdown}>
      <Markdown>{content}</Markdown>
    </Box>
  </Box>
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
      <Box className={styles.root}>
        <Box className={styles.header}>
          <Flex justify="between" align="center" gap="2" wrap="wrap">
            <Text weight="medium">
              {meta.name ?? meta.slug ?? "Task document"}
            </Text>
            {meta.version && (
              <Badge color="gray" variant="soft">
                v{meta.version}
              </Badge>
            )}
          </Flex>
          <Flex gap="2" wrap="wrap" mt="2" align="center">
            {meta.slug && <Badge variant="outline">{meta.slug}</Badge>}
            {meta.kind && (
              <Badge color={kindColor(meta.kind)} variant="soft">
                {meta.kind}
              </Badge>
            )}
            {meta.pinned && <PinStar pinned={parsePinned(meta.pinned)} />}
          </Flex>
        </Box>
        <Box className={styles.markdown}>
          <Markdown>{body || content}</Markdown>
        </Box>
      </Box>
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
    <Box className={styles.root}>
      <Flex justify="between" align="center" gap="2" className={styles.header}>
        <Text weight="medium">Task documents</Text>
        <Text size="1" color="gray">
          {rows.length} documents
        </Text>
      </Flex>
      <Flex direction="column" gap="2">
        {rows.map((row) => (
          <Box key={row.slug} className={styles.row}>
            <Flex align="center" justify="between" gap="3">
              <Flex align="center" gap="2" className={styles.identity}>
                <PinStar pinned={row.pinned} slug={row.slug} />
                <Box className={styles.identityText}>
                  <Text as="div" size="2" weight="medium">
                    {row.name}
                  </Text>
                  <Text as="div" size="1" color="gray">
                    {row.slug}
                  </Text>
                </Box>
              </Flex>
              <Flex align="center" gap="2" wrap="wrap" justify="end">
                <Badge color={kindColor(row.kind)} variant="soft">
                  {row.kind}
                </Badge>
                <Badge color="gray" variant="soft">
                  v{row.version}
                </Badge>
                <Text size="1" color="gray" className={styles.updatedAt}>
                  {row.updated_at}
                </Text>
              </Flex>
            </Flex>
          </Box>
        ))}
      </Flex>
    </Box>
  );
};

export const TaskDocumentsView: React.FC<TaskDocumentsToolProps> = ({
  toolCall,
  toolType,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
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
    <div data-testid="task-documents-view" style={{ display: "contents" }}>
    <ToolCard
      icon={<FileTextIcon />}
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
    </div>
  );
};

export default TaskDocumentsView;
