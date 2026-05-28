import React, { useCallback, useMemo } from "react";
import { Card, Flex, Spinner, Text } from "@radix-ui/themes";
import { ChatBubbleIcon } from "@radix-ui/react-icons";
import { useAppDispatch } from "../../hooks";
import { openExistingBuddyChat } from "../Chat/Thread";
import { useGetBuddyConversationsQuery } from "../../services/refact/buddy";
import type { BuddyConversationEntry } from "./types";
import styles from "./AutonomousChats.module.css";

type WorkflowGroup = {
  workflowId: string;
  entries: BuddyConversationEntry[];
};

function workflowIdFor(entry: BuddyConversationEntry): string {
  return entry.workflow_id ?? entry.badge ?? "workflow";
}

function workflowTitle(workflowId: string): string {
  return workflowId.replace(/_/g, " ");
}

function relativeTime(ts: string): string {
  if (!ts) return "";
  const diff = Date.now() - new Date(ts).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

function groupAutonomousChatsByWorkflowId(
  entries: BuddyConversationEntry[],
): WorkflowGroup[] {
  const groups = new Map<string, BuddyConversationEntry[]>();
  for (const entry of entries) {
    const workflowId = workflowIdFor(entry);
    const group = groups.get(workflowId) ?? [];
    group.push(entry);
    groups.set(workflowId, group);
  }
  return Array.from(groups, ([workflowId, groupEntries]) => ({
    workflowId,
    entries: groupEntries,
  }));
}

interface AutonomousChatsProps {
  conversations?: BuddyConversationEntry[];
}

export const AutonomousChats: React.FC<AutonomousChatsProps> = ({
  conversations,
}) => {
  const dispatch = useAppDispatch();
  const { data, isLoading } = useGetBuddyConversationsQuery(
    { kind: "workflow" },
    {
      skip: conversations !== undefined,
      refetchOnMountOrArgChange: true,
    },
  );

  const groups = useMemo(
    () => groupAutonomousChatsByWorkflowId(conversations ?? data ?? []),
    [conversations, data],
  );

  const handleOpen = useCallback(
    (entry: BuddyConversationEntry) => {
      void dispatch(openExistingBuddyChat(entry));
    },
    [dispatch],
  );

  return (
    <Flex direction="column" gap="3" data-testid="autonomous-chats">
      <Flex direction="column" gap="1">
        <Text size="2" weight="bold">
          Autonomous chats
        </Text>
        <Text size="1" color="gray">
          Workflow-driven Buddy conversations grouped by workflow.
        </Text>
      </Flex>

      {isLoading && conversations === undefined && (
        <Flex align="center" justify="center" py="3">
          <Spinner size="2" />
        </Flex>
      )}

      {!isLoading && groups.length === 0 && (
        <Card className={styles.emptyCard}>
          <Flex align="center" gap="2">
            <ChatBubbleIcon width={16} height={16} />
            <Text size="1" color="gray">
              No autonomous chats yet
            </Text>
          </Flex>
        </Card>
      )}

      {groups.map((group) => (
        <Card key={group.workflowId} className={styles.groupCard}>
          <Flex direction="column" gap="2">
            <Flex align="center" justify="between" gap="2">
              <Flex align="center" gap="2" minWidth="0">
                <Text size="3" className={styles.groupIcon}>
                  ⚙️
                </Text>
                <Text size="2" weight="bold" className={styles.groupTitle}>
                  {workflowTitle(group.workflowId)}
                </Text>
              </Flex>
              <Text size="1" color="gray" className={styles.groupCount}>
                {group.entries.length} chats
              </Text>
            </Flex>
            <div className={styles.chatList}>
              {group.entries.map((entry) => (
                <button
                  key={entry.id}
                  type="button"
                  className={styles.chatRow}
                  onClick={() => handleOpen(entry)}
                >
                  <span className={styles.chatIcon}>{entry.icon}</span>
                  <span className={styles.chatContent}>
                    <span className={styles.chatTitle}>
                      {entry.title || "Untitled"}
                    </span>
                    <span className={styles.chatMeta}>
                      {entry.message_count} entries · {entry.status}
                    </span>
                  </span>
                  <span className={styles.chatTime}>
                    {relativeTime(entry.updated_at)}
                  </span>
                </button>
              ))}
            </div>
          </Flex>
        </Card>
      ))}
    </Flex>
  );
};
