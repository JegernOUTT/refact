import React, { useCallback, useMemo, useState } from "react";
import {
  Badge,
  Box,
  Button,
  Card,
  Flex,
  IconButton,
  Popover,
  Spinner,
  Text,
  Tooltip,
} from "@radix-ui/themes";
import {
  ArchiveIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  DrawingPinIcon,
} from "@radix-ui/react-icons";
import classNames from "classnames";
import type { TaskMemoryEntry } from "../../../services/refact/taskMemoriesApi";
import styles from "./MemoryInboxPanel.module.css";

const KIND_COLORS: Record<
  TaskMemoryEntry["kind"],
  "blue" | "green" | "amber" | "red" | "purple" | "gray"
> = {
  decision: "purple",
  spec: "blue",
  finding: "green",
  gotcha: "amber",
  risk: "red",
  handoff: "purple",
  progress: "blue",
  postmortem: "amber",
  brief: "green",
  freeform: "gray",
};

const MAX_COLLAPSED_TAGS = 3;
const PREVIEW_LENGTH = 180;

type MemoryCardProps = {
  memory: TaskMemoryEntry;
  onPin: (filename: string, pinned: boolean) => void | Promise<void>;
  onArchive: (filename: string) => void | Promise<void>;
  disabled?: boolean;
  pending?: boolean;
};

function buildPreview(content: string): string {
  const trimmed = content.trim();
  if (trimmed.length <= PREVIEW_LENGTH) return trimmed;
  return `${trimmed.slice(0, PREVIEW_LENGTH).trimEnd()}…`;
}

export const MemoryCard: React.FC<MemoryCardProps> = ({
  memory,
  onPin,
  onArchive,
  disabled = false,
  pending = false,
}) => {
  const [expanded, setExpanded] = useState(false);
  const [tagsExpanded, setTagsExpanded] = useState(false);

  const handlePin = useCallback(() => {
    void onPin(memory.filename, !memory.pinned);
  }, [memory.filename, memory.pinned, onPin]);

  const handleArchive = useCallback(() => {
    void onArchive(memory.filename);
  }, [memory.filename, onArchive]);

  const createdAt = memory.created_at_known
    ? new Date(memory.created_at).toLocaleString()
    : "unknown time";
  const title = memory.title.trim() || memory.filename;
  const content = memory.content.trim() || "No content";
  const preview = useMemo(() => buildPreview(content), [content]);
  const canExpand = preview !== content;
  const visibleTags = tagsExpanded
    ? memory.tags
    : memory.tags.slice(0, MAX_COLLAPSED_TAGS);
  const hiddenTagCount = memory.tags.length - visibleTags.length;

  return (
    <Card
      className={classNames(styles.card, memory.pinned && styles.cardPinned)}
      data-testid={`memory-card-${memory.filename}`}
    >
      <Flex direction="column" gap="2">
        <Flex justify="between" align="start" gap="2">
          <Flex gap="1" align="center" wrap="wrap">
            <Badge color={KIND_COLORS[memory.kind]} variant="soft">
              {memory.kind}
            </Badge>
            {memory.pinned && (
              <Badge color="amber" variant="solid">
                pinned
              </Badge>
            )}
            <Text size="1" color="gray" className={styles.cardMetaText}>
              {memory.namespace}
            </Text>
          </Flex>
          <Flex gap="1" align="center" className={styles.cardControls}>
            <Tooltip content={memory.pinned ? "Unpin" : "Pin memory"}>
              <IconButton
                size="1"
                variant="ghost"
                aria-label={memory.pinned ? "Unpin" : "Pin"}
                color={memory.pinned ? "amber" : "gray"}
                onClick={handlePin}
                disabled={disabled}
                className={styles.cardIconButton}
              >
                <DrawingPinIcon />
              </IconButton>
            </Tooltip>
            <Popover.Root>
              <Tooltip content="Archive">
                <Popover.Trigger>
                  <IconButton
                    size="1"
                    variant="ghost"
                    aria-label="Archive"
                    color="gray"
                    disabled={disabled}
                    className={styles.cardIconButton}
                  >
                    <ArchiveIcon />
                  </IconButton>
                </Popover.Trigger>
              </Tooltip>
              <Popover.Content width="220px">
                <Flex direction="column" gap="3">
                  <Text size="2">Archive this memory?</Text>
                  <Flex gap="2">
                    <Popover.Close>
                      <Button
                        size="1"
                        variant="solid"
                        color="amber"
                        onClick={handleArchive}
                      >
                        Confirm archive
                      </Button>
                    </Popover.Close>
                    <Popover.Close>
                      <Button size="1" variant="soft" color="gray">
                        Cancel
                      </Button>
                    </Popover.Close>
                  </Flex>
                </Flex>
              </Popover.Content>
            </Popover.Root>
            {canExpand && (
              <Tooltip
                content={expanded ? "Collapse preview" : "Expand preview"}
              >
                <IconButton
                  size="1"
                  variant="ghost"
                  aria-label={expanded ? "Collapse" : "Expand"}
                  color="gray"
                  onClick={() => setExpanded((v) => !v)}
                  aria-expanded={expanded}
                  className={styles.cardIconButton}
                >
                  {expanded ? <ChevronUpIcon /> : <ChevronDownIcon />}
                </IconButton>
              </Tooltip>
            )}
          </Flex>
        </Flex>

        <Text weight="medium" size="2" className={styles.cardTitle}>
          {title}
        </Text>

        <Box
          className={classNames(
            styles.preview,
            expanded && styles.previewExpanded,
          )}
        >
          <Text as="div" size="2" color="gray">
            {expanded ? content : preview}
          </Text>
        </Box>

        <Flex justify="between" align="start" gap="2">
          {memory.tags.length > 0 ? (
            <Flex gap="1" wrap="wrap" align="center" className={styles.tags}>
              {visibleTags.map((tag) => (
                <Badge key={tag} color="gray" variant="outline">
                  {tag}
                </Badge>
              ))}
              {hiddenTagCount > 0 && (
                <Button
                  type="button"
                  size="1"
                  variant="ghost"
                  className={styles.tagsToggle}
                  onClick={() => setTagsExpanded(true)}
                >
                  Show {hiddenTagCount} more
                </Button>
              )}
              {tagsExpanded && memory.tags.length > MAX_COLLAPSED_TAGS && (
                <Button
                  type="button"
                  size="1"
                  variant="ghost"
                  className={styles.tagsToggle}
                  onClick={() => setTagsExpanded(false)}
                >
                  Show fewer
                </Button>
              )}
            </Flex>
          ) : (
            <Box />
          )}
          <Flex align="center" gap="2" className={styles.cardFooterRight}>
            {pending && (
              <Flex align="center" gap="1" className={styles.pendingState}>
                <Spinner size="1" />
                <Text size="1" color="gray">
                  Updating
                </Text>
              </Flex>
            )}
            <Text size="1" color="gray" className={styles.cardDate}>
              {createdAt}
            </Text>
          </Flex>
        </Flex>
      </Flex>
    </Card>
  );
};

export default MemoryCard;
