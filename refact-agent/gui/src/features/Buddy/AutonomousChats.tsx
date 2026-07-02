import React, { useCallback, useMemo } from "react";
import { MessageCircle } from "lucide-react";
import { Icon, LoadingState, Surface, Text } from "../../components/ui";
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
  return entry.badge ?? "workflow";
}

function workflowTitle(workflowId: string): string {
  return workflowId.replace(/_/g, " ");
}

function relativeTime(ts: string): string {
  if (!ts) return "";
  const time = new Date(ts).getTime();
  if (!Number.isFinite(time)) return "unknown";
  const diff = Date.now() - time;
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
    <section className={styles.root} data-testid="autonomous-chats">
      <div className={styles.header}>
        <Text as="strong" size="2" weight="bold">
          Autonomous chats
        </Text>
        <Text as="p" size="1" color="gray">
          Workflow-driven Buddy conversations grouped by workflow.
        </Text>
      </div>

      {isLoading && conversations === undefined && (
        <LoadingState label="Loading autonomous chats" variant="compact" />
      )}

      {!isLoading && groups.length === 0 && (
        <Surface className={styles.emptyCard} variant="plain" radius="card">
          <Icon icon={MessageCircle} size="md" tone="muted" />
          <Text size="1" color="gray">
            No autonomous chats yet
          </Text>
        </Surface>
      )}

      {groups.map((group) => (
        <Surface
          key={group.workflowId}
          className={styles.groupCard}
          radius="card"
          variant="plain"
        >
          <div className={styles.groupHeader}>
            <div className={styles.groupTitleWrap}>
              <span className={styles.groupIcon}>⚙️</span>
              <Text
                as="strong"
                size="2"
                weight="bold"
                className={styles.groupTitle}
              >
                {workflowTitle(group.workflowId)}
              </Text>
            </div>
            <Text size="1" color="gray" className={styles.groupCount}>
              {group.entries.length} chats
            </Text>
          </div>
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
        </Surface>
      ))}
    </section>
  );
};
