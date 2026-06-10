import React, { useCallback, useMemo, useState } from "react";
import classNames from "classnames";
import { Pin, Trash2 } from "lucide-react";
import { Markdown } from "../../../components/Markdown";
import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../../../components/shared/useDelayedUnmount";
import {
  Badge,
  Button,
  Flex,
  IconButton,
  Popover,
  Spinner,
  Surface,
  Text,
  Tooltip,
} from "../../../components/ui";
import type { BadgeTone } from "../../../components/ui";
import type { TaskMemoryEntry } from "../../../services/refact/taskMemoriesApi";
import { memoryKindColor } from "../../../services/refact/taskKinds";
import styles from "./MemoryInboxPanel.module.css";

const TITLE_FALLBACK_LENGTH = 80;
const PREVIEW_LENGTH = 180;

type MemoryCardProps = {
  memory: TaskMemoryEntry;
  onPin: (filename: string, pinned: boolean) => void | Promise<void>;
  onArchive: (filename: string) => void | Promise<void>;
  disabled?: boolean;
  pending?: boolean;
  expanded?: boolean;
  onExpandedChange?: (filename: string, expanded: boolean) => void;
};

type TitleInfo = {
  text: string;
  empty: boolean;
};

function normalizeLine(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function truncateTitleFallback(value: string): string {
  if (value.length <= TITLE_FALLBACK_LENGTH) return value;
  return `${value.slice(0, TITLE_FALLBACK_LENGTH).trimEnd()}…`;
}

function buildTitle(memory: TaskMemoryEntry): TitleInfo {
  const title = normalizeLine(memory.title);
  if (title) return { text: title, empty: false };

  const contentLine = memory.content
    .split(/\r?\n/)
    .map(normalizeLine)
    .find((line) => line.length > 0);
  if (contentLine) {
    return { text: truncateTitleFallback(contentLine), empty: false };
  }

  return { text: "(no title)", empty: true };
}

function buildPreview(content: string): string {
  const normalized = normalizeLine(content);
  if (normalized.length <= PREVIEW_LENGTH) return normalized;
  return `${normalized.slice(0, PREVIEW_LENGTH).trimEnd()}…`;
}

function frontmatterRows(
  memory: TaskMemoryEntry,
): { label: string; value: string }[] {
  return [
    { label: "kind", value: memory.kind },
    { label: "namespace", value: memory.namespace },
    { label: "created_at", value: memory.created_at },
    { label: "pinned", value: memory.pinned ? "true" : "false" },
    { label: "supersedes", value: memory.supersedes?.trim() ?? "—" },
  ];
}

function badgeTone(color: ReturnType<typeof memoryKindColor>): BadgeTone {
  if (color === "red") return "danger";
  if (color === "amber") return "warning";
  if (color === "gray") return "muted";
  return "accent";
}

export const MemoryCard: React.FC<MemoryCardProps> = ({
  memory,
  onPin,
  onArchive,
  disabled = false,
  pending = false,
  expanded,
  onExpandedChange,
}) => {
  const [localExpanded, setLocalExpanded] = useState(false);
  const isExpanded = expanded ?? localExpanded;
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isExpanded,
    COLLAPSE_ANIMATION_MS,
  );
  const shouldRenderExpanded = isExpanded || shouldRender;
  const title = useMemo(() => buildTitle(memory), [memory]);
  const content = memory.content.trim();
  const preview = useMemo(() => buildPreview(memory.content), [memory.content]);
  const createdAt = memory.created_at_known
    ? new Date(memory.created_at).toLocaleString()
    : "unknown time";

  const setExpanded = useCallback(
    (next: boolean) => {
      if (expanded === undefined) {
        setLocalExpanded(next);
      }
      onExpandedChange?.(memory.filename, next);
    },
    [expanded, memory.filename, onExpandedChange],
  );

  const handleToggleExpanded = useCallback(() => {
    setExpanded(!isExpanded);
  }, [isExpanded, setExpanded]);

  const handlePin = useCallback(() => {
    void onPin(memory.filename, !memory.pinned);
  }, [memory.filename, memory.pinned, onPin]);

  const handleArchive = useCallback(() => {
    void onArchive(memory.filename);
  }, [memory.filename, onArchive]);

  return (
    <Surface
      animated="rise"
      className={classNames(memory.pinned && styles.cardPinned, styles.card)}
      data-expanded={isExpanded ? "true" : "false"}
      data-testid={`memory-card-${memory.filename}`}
      radius="card"
      variant="plain"
    >
      <Flex direction="column" gap="2" className={styles.cardFrame}>
        <Flex align="start" gap="2" className={styles.cardCollapsedRow}>
          <button
            type="button"
            className={styles.cardBodyButton}
            onClick={handleToggleExpanded}
            aria-expanded={isExpanded}
            aria-label={`${isExpanded ? "Collapse" : "Expand"} memory ${
              title.text
            }`}
          >
            <Flex direction="column" gap="1" className={styles.cardBodyColumn}>
              <Flex align="center" gap="2" className={styles.cardTitleRow}>
                <Flex gap="1" align="center" className={styles.cardBadges}>
                  <Badge tone={badgeTone(memoryKindColor(memory.kind))}>
                    {memory.kind}
                  </Badge>
                  <Badge tone="muted">{memory.namespace}</Badge>
                </Flex>
                <Text
                  weight="medium"
                  size="2"
                  className={classNames(
                    styles.cardTitle,
                    title.empty && styles.cardTitleEmpty,
                  )}
                >
                  {title.text}
                </Text>
              </Flex>

              <Flex
                align="end"
                justify="between"
                gap="2"
                className={styles.cardPreviewRow}
              >
                {preview ? (
                  <Text size="1" className={styles.cardPreviewMuted}>
                    {preview}
                  </Text>
                ) : (
                  <div className={styles.cardPreviewEmpty} />
                )}
                <Text size="1" className={styles.cardDate}>
                  {createdAt}
                </Text>
              </Flex>
            </Flex>
          </button>

          <Flex
            direction="column"
            align="end"
            gap="1"
            className={styles.cardControls}
          >
            <Flex gap="1" align="center">
              <Tooltip content={memory.pinned ? "Unpin" : "Pin memory"}>
                <IconButton
                  size="sm"
                  variant="plain"
                  aria-label={memory.pinned ? "Unpin" : "Pin"}
                  icon={Pin}
                  onClick={handlePin}
                  disabled={disabled}
                  className={classNames(
                    styles.cardIconButton,
                    memory.pinned && styles.iconButtonPinned,
                  )}
                />
              </Tooltip>
              <Popover>
                <Tooltip content="Archive">
                  <Popover.Trigger asChild>
                    <IconButton
                      size="sm"
                      variant="plain"
                      aria-label="Archive"
                      icon={Trash2}
                      disabled={disabled}
                      className={styles.cardIconButton}
                    />
                  </Popover.Trigger>
                </Tooltip>
                <Popover.Content className={styles.archivePopover}>
                  <Flex direction="column" gap="3">
                    <Text size="2">Archive this memory?</Text>
                    <Flex gap="2" wrap="wrap">
                      <Popover.Close asChild>
                        <Button
                          size="sm"
                          variant="soft"
                          onClick={handleArchive}
                        >
                          Confirm archive
                        </Button>
                      </Popover.Close>
                      <Popover.Close asChild>
                        <Button size="sm" variant="plain">
                          Cancel
                        </Button>
                      </Popover.Close>
                    </Flex>
                  </Flex>
                </Popover.Content>
              </Popover>
            </Flex>
            {pending && (
              <Flex align="center" gap="1" className={styles.pendingState}>
                <Spinner size="sm" />
                <Text size="1" className={styles.cardPreviewMuted}>
                  Updating
                </Text>
              </Flex>
            )}
          </Flex>
        </Flex>

        {shouldRenderExpanded && (
          <div
            className={classNames("rf-expand-grid", styles.expandedGrid)}
            data-open={isAnimatingOpen}
          >
            <div
              className={styles.expandedContent}
              data-testid={`memory-card-expanded-${memory.filename}`}
            >
              {content ? (
                <div className={styles.expandedMarkdown}>
                  <Markdown canHaveInteractiveElements={false}>
                    {content}
                  </Markdown>
                </div>
              ) : (
                <Text size="2" className={styles.emptyContent}>
                  No content
                </Text>
              )}

              <Flex
                gap="1"
                wrap="wrap"
                align="center"
                className={styles.expandedTags}
              >
                {memory.tags.length > 0 ? (
                  memory.tags.map((tag) => (
                    <Badge key={tag} tone="muted">
                      {tag}
                    </Badge>
                  ))
                ) : (
                  <Text size="1" className={styles.cardPreviewMuted}>
                    No tags
                  </Text>
                )}
              </Flex>

              <table
                className={styles.frontmatterTable}
                data-testid={`memory-card-frontmatter-${memory.filename}`}
              >
                <tbody>
                  {frontmatterRows(memory).map((row) => (
                    <tr key={row.label}>
                      <th scope="row">{row.label}</th>
                      <td>{row.value}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </Flex>
    </Surface>
  );
};

export default MemoryCard;
