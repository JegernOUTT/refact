import React, { useCallback, useState } from "react";
import {
  Badge,
  Box,
  Button,
  Card,
  Dialog,
  Flex,
  Spinner,
  Text,
} from "@radix-ui/themes";
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

type MemoryCardProps = {
  memory: TaskMemoryEntry;
  onPin: (filename: string, pinned: boolean) => void | Promise<void>;
  onArchive: (filename: string) => void | Promise<void>;
  disabled?: boolean;
  pending?: boolean;
};

export const MemoryCard: React.FC<MemoryCardProps> = ({
  memory,
  onPin,
  onArchive,
  disabled = false,
  pending = false,
}) => {
  const [viewOpen, setViewOpen] = useState(false);

  const handlePin = useCallback(() => {
    void onPin(memory.filename, !memory.pinned);
  }, [memory.filename, memory.pinned, onPin]);

  const handleArchive = useCallback(() => {
    void onArchive(memory.filename);
  }, [memory.filename, onArchive]);

  const createdAt = memory.created_at_known
    ? new Date(memory.created_at).toLocaleString()
    : "unknown time";

  return (
    <Card
      className={classNames(styles.card, memory.pinned && styles.cardPinned)}
      data-testid={`memory-card-${memory.filename}`}
    >
      <Flex direction="column" gap="2">
        <Flex justify="between" align="start" gap="2" className={styles.cardHeader}>
          <Flex direction="column" gap="1" className={styles.cardHeader}>
            <Flex gap="2" align="center" className={styles.cardHeader}>
              <Badge color={KIND_COLORS[memory.kind]} variant="soft">
                {memory.kind}
              </Badge>
              {memory.pinned && (
                <Badge color="amber" variant="solid">
                  pinned
                </Badge>
              )}
              <Text size="1" color="gray">
                {memory.namespace}
              </Text>
            </Flex>
            <Text weight="medium" size="2" className={styles.cardTitle}>
              {memory.title || memory.filename}
            </Text>
          </Flex>
          <Text size="1" color="gray">
            {createdAt}
          </Text>
        </Flex>

        <Text size="2" color="gray" className={styles.excerpt}>
          {memory.content || "No content"}
        </Text>

        {memory.tags.length > 0 && (
          <Flex gap="1" wrap="wrap" className={styles.tags}>
            {memory.tags.map((tag) => (
              <Badge key={tag} color="gray" variant="outline">
                {tag}
              </Badge>
            ))}
          </Flex>
        )}

        <Flex gap="2" wrap="wrap" className={styles.actions}>
          <Button size="1" variant="soft" onClick={handlePin} disabled={disabled}>
            {memory.pinned ? "Unpin" : "Pin"}
          </Button>
          <Button
            size="1"
            variant="soft"
            color="amber"
            onClick={handleArchive}
            disabled={disabled}
          >
            Archive
          </Button>
          <Button size="1" variant="ghost" onClick={() => setViewOpen(true)}>
            View full
          </Button>
          {pending && (
            <Flex align="center" gap="1" className={styles.pendingState}>
              <Spinner size="1" />
              <Text size="1" color="gray">
                Updating
              </Text>
            </Flex>
          )}
        </Flex>
      </Flex>

      <Dialog.Root open={viewOpen} onOpenChange={setViewOpen}>
        <Dialog.Content className={styles.dialogContent}>
          <Dialog.Title>{memory.title || memory.filename}</Dialog.Title>
          <Dialog.Description size="2" color="gray">
            {memory.filename}
          </Dialog.Description>
          <Box className={styles.fullContent} mt="3">
            <Text as="div" size="2">
              {memory.content || "No content"}
            </Text>
          </Box>
          <Flex justify="end" mt="3">
            <Dialog.Close>
              <Button variant="soft" color="gray">
                Close
              </Button>
            </Dialog.Close>
          </Flex>
        </Dialog.Content>
      </Dialog.Root>
    </Card>
  );
};

export default MemoryCard;
