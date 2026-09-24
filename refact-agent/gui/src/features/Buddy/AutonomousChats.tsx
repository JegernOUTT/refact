import React, { useCallback, useMemo, useState } from "react";
import { AlertTriangle, MessageCircle } from "lucide-react";
import {
  Badge,
  Button,
  Icon,
  LoadingState,
  Surface,
  Text,
} from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { humanizeIdentifier } from "../../utils/displayNames";
import { openExistingBuddyChat } from "../Chat/Thread";
import { useGetBuddyConversationsQuery } from "../../services/refact/buddy";
import { formatRelativeTime } from "./buddyUtils";
import {
  collapseConsecutive,
  countLabel,
  HISTORY_PAGE_SIZE,
} from "./buddyHistory";
import type { BuddyConversationEntry, BuddyLedgerDiagnostics } from "./types";
import styles from "./AutonomousChats.module.css";

function corruptionCount(
  diagnostics: BuddyLedgerDiagnostics | null | undefined,
): number {
  if (!diagnostics) return 0;
  return (
    diagnostics.invalid_json + diagnostics.missing_id + diagnostics.quarantined
  );
}

function workflowIdFor(entry: BuddyConversationEntry): string {
  return entry.badge ?? "workflow";
}

function workflowLabel(entry: BuddyConversationEntry): string {
  const label = entry.badge ?? "Workflow";
  return /[_-]/.test(label) ? humanizeIdentifier(label) : label;
}

function workflowEntryTitle(entry: BuddyConversationEntry): string {
  const legacyTitle = `${entry.id.replace(/_/g, " ")} (${entry.badge})`;
  return entry.badge && entry.title === legacyTitle
    ? entry.badge
    : entry.title || "Untitled";
}

interface AutonomousChatsProps {
  conversations?: BuddyConversationEntry[];
}

export const AutonomousChats: React.FC<AutonomousChatsProps> = ({
  conversations,
}) => {
  const [visibleCount, setVisibleCount] = useState(HISTORY_PAGE_SIZE);
  const dispatch = useAppDispatch();
  const { data, isLoading } = useGetBuddyConversationsQuery(
    { kind: "workflow" },
    {
      skip: conversations !== undefined,
      refetchOnMountOrArgChange: true,
    },
  );
  const allGroups = useMemo(() => {
    const groups = new Map<
      string,
      { entry: BuddyConversationEntry; count: number }[]
    >();
    for (const item of collapseConsecutive(
      conversations ?? data?.entries ?? [],
      (entry) => `${workflowIdFor(entry)}\u0000${entry.title}`,
    )) {
      const workflowId = workflowIdFor(item.entry);
      const group = groups.get(workflowId) ?? [];
      group.push(item);
      groups.set(workflowId, group);
    }
    return Array.from(groups, ([workflowId, entries]) => ({
      workflowId,
      entries,
    }));
  }, [conversations, data]);
  let remaining = visibleCount;
  const groups = allGroups
    .map((group) => {
      const entries = group.entries.slice(0, remaining);
      remaining -= entries.length;
      return { ...group, entries };
    })
    .filter((group) => group.entries.length > 0);
  const shownCount = visibleCount - remaining;
  const totalCount = allGroups.reduce(
    (total, group) => total + group.entries.length,
    0,
  );
  const corrupted = corruptionCount(data?.diagnostics);
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
        {corrupted > 0 && (
          <Badge
            tone="warning"
            size="xs"
            data-testid="ledger-corruption-chip"
            title="Corrupt ledger files were quarantined with a .corrupt suffix"
          >
            <Icon icon={AlertTriangle} size="sm" />
            {corrupted} corrupt file{corrupted === 1 ? "" : "s"} quarantined
          </Badge>
        )}
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

      <ul className={styles.groupList}>
        {groups.map((group) => {
          const groupTitle = workflowLabel(group.entries[0].entry);
          return (
            <li key={group.workflowId}>
              <Surface
                as="section"
                aria-label={groupTitle}
                className={styles.groupCard}
                radius="card"
                variant="plain"
              >
                <div className={styles.groupHeader}>
                  <div className={styles.groupTitleWrap}>
                    <span className={styles.groupIcon}>⚙️</span>
                    <Text
                      as="h3"
                      size="2"
                      weight="bold"
                      className={styles.groupTitle}
                    >
                      {groupTitle}
                    </Text>
                  </div>
                  <Text size="1" color="gray" className={styles.groupCount}>
                    {countLabel(
                      group.entries.reduce(
                        (total, item) => total + item.count,
                        0,
                      ),
                      "chat",
                      "chats",
                    )}
                  </Text>
                </div>
                <ul className={styles.chatList}>
                  {group.entries.map(({ entry, count }) => {
                    const title =
                      entry.kind === "workflow"
                        ? workflowEntryTitle(entry)
                        : entry.title || "Untitled";
                    return (
                      <li key={entry.id} className={styles.chatListItem}>
                        <button
                          type="button"
                          className={styles.chatRow}
                          aria-label={`Open ${title}`}
                          onClick={() => handleOpen(entry)}
                        >
                          <span className={styles.chatIcon}>{entry.icon}</span>
                          <span className={styles.chatContent}>
                            <span className={styles.chatTitle}>
                              {title}
                              {count > 1 && (
                                <span className={styles.duplicateCount}>
                                  {countLabel(count, "chat", "chats")}
                                </span>
                              )}
                            </span>
                            <span className={styles.chatMeta}>
                              {countLabel(
                                entry.message_count,
                                "entry",
                                "entries",
                              )}{" "}
                              · {entry.status}
                            </span>
                          </span>
                          <span className={styles.chatTime}>
                            {formatRelativeTime(entry.updated_at)}
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </Surface>
            </li>
          );
        })}
      </ul>
      {shownCount < totalCount && (
        <Button
          type="button"
          variant="ghost"
          onClick={() => setVisibleCount((count) => count + HISTORY_PAGE_SIZE)}
        >
          Show more
        </Button>
      )}
    </section>
  );
};
