import React, { useCallback, useState } from "react";
import { Flex, Text, Spinner } from "@radix-ui/themes";
import { ChatBubbleIcon, PlusIcon } from "@radix-ui/react-icons";
import { useAppDispatch } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { openBuddyChat, newBuddyChatAction } from "../Chat/Thread";
import {
  useGetBuddyConversationsQuery,
  useCreateBuddyConversationMutation,
} from "../../services/refact/buddy";
import type { BuddyConversationEntry } from "./types";
import styles from "./BuddyRecentChats.module.css";

type FilterKind = "all" | "chat" | "setup" | "system";

const FILTER_LABELS: { kind: FilterKind; label: string }[] = [
  { kind: "all", label: "All" },
  { kind: "chat", label: "Chats" },
  { kind: "setup", label: "Setup" },
  { kind: "system", label: "System" },
];

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

interface EntryRowProps {
  entry: BuddyConversationEntry;
  onClick: (entry: BuddyConversationEntry) => void;
}

const EntryRow: React.FC<EntryRowProps> = ({ entry, onClick }) => {
  const clickable = entry.kind === "chat" || entry.kind === "setup";
  return (
    <button
      type="button"
      className={styles.entryRow}
      onClick={clickable ? () => onClick(entry) : undefined}
      data-clickable={clickable || undefined}
    >
      <span className={styles.entryIcon}>{entry.icon}</span>
      <Flex direction="column" gap="0" style={{ flex: 1, minWidth: 0 }}>
        <Flex align="center" gap="1" style={{ minWidth: 0 }}>
          <Text size="1" weight="medium" className={styles.entryTitle} truncate>
            {entry.title || "Untitled"}
          </Text>
          {entry.badge && <span className={styles.badge}>{entry.badge}</span>}
        </Flex>
        <Flex align="center" gap="1">
          <Text size="1" color="gray" className={styles.entryMeta}>
            {entry.message_count > 0
              ? `${entry.message_count} entries`
              : entry.status}
          </Text>
          {entry.updated_at && (
            <>
              <Text size="1" color="gray">
                ·
              </Text>
              <Text size="1" color="gray" className={styles.entryMeta}>
                {relativeTime(entry.updated_at)}
              </Text>
            </>
          )}
        </Flex>
      </Flex>
    </button>
  );
};

interface BuddyRecentChatsProps {
  compact?: boolean;
  maxItems?: number;
  showFilters?: boolean;
  onViewAll?: () => void;
  title?: string;
  className?: string;
}

export const BuddyRecentChats: React.FC<BuddyRecentChatsProps> = ({
  compact = false,
  maxItems,
  showFilters = true,
  onViewAll,
  title,
  className,
}) => {
  const dispatch = useAppDispatch();
  const [filter, setFilter] = useState<FilterKind>("all");

  const { data: allConversations, isLoading } = useGetBuddyConversationsQuery(
    undefined,
    { refetchOnMountOrArgChange: true },
  );
  const [createConversation, { isLoading: isCreating }] =
    useCreateBuddyConversationMutation();

  const conversations = React.useMemo(() => {
    if (!allConversations) return [];
    const filtered =
      filter === "all"
        ? allConversations
        : filter === "system"
          ? allConversations.filter(
              (e) => e.kind === "system" || e.kind === "workflow",
            )
          : allConversations.filter((e) => e.kind === filter);
    return maxItems ? filtered.slice(0, maxItems) : filtered;
  }, [allConversations, filter, maxItems]);

  const handleOpen = useCallback(
    (entry: BuddyConversationEntry) => {
      dispatch(openBuddyChat({ chat_id: entry.id, title: entry.title }));
      dispatch(push({ name: "chat" }));
    },
    [dispatch],
  );

  const handleNew = useCallback(async () => {
    const result = await createConversation(undefined);
    if ("data" in result && result.data) {
      const meta = result.data;
      dispatch(newBuddyChatAction({ chat_id: meta.chat_id }));
      dispatch(openBuddyChat({ chat_id: meta.chat_id, title: meta.title }));
      dispatch(push({ name: "chat" }));
    }
  }, [createConversation, dispatch]);

  return (
    <Flex direction="column" gap="2" className={className}>
      <Flex align="center" justify="between">
        <Text
          size="1"
          weight="bold"
          color="gray"
          className={styles.sectionLabel}
        >
          {title ?? (compact ? "RECENT ACTIVITY" : "CONVERSATIONS")}
        </Text>
        <Flex align="center" gap="1">
          {onViewAll && (
            <button
              type="button"
              className={styles.headerChip}
              onClick={onViewAll}
            >
              View All →
            </button>
          )}
          {!compact && (
            <button
              type="button"
              className={styles.headerChip}
              onClick={() => void handleNew()}
              disabled={isCreating}
            >
              {isCreating ? (
                <Spinner size="1" />
              ) : (
                <PlusIcon width={12} height={12} />
              )}
              New Chat
            </button>
          )}
        </Flex>
      </Flex>

      {showFilters && !compact && (
        <Flex gap="1" className={styles.filterTabs}>
          {FILTER_LABELS.map(({ kind, label }) => (
            <button
              key={kind}
              type="button"
              className={styles.filterTab}
              data-active={filter === kind || undefined}
              onClick={() => setFilter(kind)}
            >
              <Text size="1">{label}</Text>
            </button>
          ))}
        </Flex>
      )}

      {isLoading && (
        <Flex align="center" justify="center" py="3">
          <Spinner size="2" />
        </Flex>
      )}

      {!isLoading && conversations.length === 0 && (
        <Flex
          direction="column"
          align="center"
          justify="center"
          gap="2"
          py="4"
          className={styles.empty}
        >
          <ChatBubbleIcon width={20} height={20} />
          <Text size="1" color="gray">
            {filter === "all" ? "No conversations yet" : `No ${filter} entries`}
          </Text>
          {filter === "all" && (
            <button
              type="button"
              className={styles.emptyChip}
              onClick={() => void handleNew()}
            >
              Start a conversation
            </button>
          )}
        </Flex>
      )}

      {conversations.length > 0 && (
        <div className={styles.entriesScroll}>
          {conversations.map((entry) => (
            <EntryRow
              key={`${entry.kind}-${entry.id}`}
              entry={entry}
              onClick={handleOpen}
            />
          ))}
        </div>
      )}
    </Flex>
  );
};
