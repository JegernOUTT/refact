import React, { useCallback, useState } from "react";
import classNames from "classnames";
import { MessagesSquare, Plus } from "lucide-react";
import {
  Badge,
  Button,
  Icon,
  LoadingState,
  SegmentedControl,
  Surface,
  Text,
} from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import {
  openBuddyChat,
  newBuddyChatAction,
  openExistingBuddyChat,
} from "../Chat/Thread";
import {
  useGetBuddyConversationsQuery,
  useCreateBuddyConversationMutation,
} from "../../services/refact/buddy";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { conversationIcon } from "./buddyIcons";
import type { BuddyConversationEntry } from "./types";
import styles from "./BuddyRecentChats.module.css";

type FilterKind = "all" | "chat" | "setup" | "system";

const FILTER_OPTIONS: { value: FilterKind; label: string }[] = [
  { value: "all", label: "All" },
  { value: "chat", label: "Chats" },
  { value: "setup", label: "Setup" },
  { value: "system", label: "System" },
];

const STALE_EMPTY_PLACEHOLDER_MS = 24 * 60 * 60 * 1000;

function relativeTime(ts: string): string {
  if (!ts) return "";
  const time = Date.parse(ts);
  if (!Number.isFinite(time)) return "";
  const diff = Date.now() - time;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.floor(hrs / 24)}d ago`;
}

function isStaleEmptyPlaceholder(entry: BuddyConversationEntry): boolean {
  if (entry.kind !== "chat") return false;
  if (entry.message_count > 0) return false;
  if ((entry.title || "").trim() !== "New Conversation") return false;
  const timestamp = Date.parse(entry.updated_at || entry.created_at);
  if (!Number.isFinite(timestamp)) return false;
  return Date.now() - timestamp > STALE_EMPTY_PLACEHOLDER_MS;
}

interface EntryRowProps {
  entry: BuddyConversationEntry;
  onClick: (entry: BuddyConversationEntry) => void;
}

const EntryRow: React.FC<EntryRowProps> = ({ entry, onClick }) => {
  const clickable =
    entry.kind === "chat" ||
    entry.kind === "setup" ||
    entry.kind === "workflow";
  const content = (
    <>
      <span className={styles.entryIcon}>
        <Icon icon={conversationIcon(entry.kind)} size="sm" tone="muted" />
      </span>
      <span className={styles.entryBody}>
        <span className={styles.entryTitleRow}>
          <Text size="1" weight="medium" className={styles.entryTitle}>
            {entry.title || "Untitled"}
          </Text>
          {entry.badge && (
            <Badge size="xs" tone="muted" className={styles.entryBadge}>
              {entry.badge}
            </Badge>
          )}
        </span>
        <Text size="1" color="gray" className={styles.entryMeta}>
          {entry.message_count > 0
            ? `${entry.message_count} entries`
            : entry.status}
          {entry.updated_at ? ` · ${relativeTime(entry.updated_at)}` : ""}
        </Text>
      </span>
    </>
  );
  if (!clickable) {
    return (
      <div className={classNames(styles.entryRow, "rf-enter-rise")}>
        {content}
      </div>
    );
  }
  return (
    <button
      type="button"
      className={classNames(styles.entryRow, "rf-enter-rise", "rf-pressable")}
      onClick={() => onClick(entry)}
      data-clickable
    >
      {content}
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

  const { data: conversationsList, isLoading } = useGetBuddyConversationsQuery(
    undefined,
    { refetchOnMountOrArgChange: true },
  );
  const allConversations = conversationsList?.entries;
  const [createConversation, { isLoading: isCreating }] =
    useCreateBuddyConversationMutation();

  const conversations = React.useMemo(() => {
    if (!allConversations) return [];
    const visibleConversations = allConversations.filter(
      (entry) => !isStaleEmptyPlaceholder(entry),
    );
    const filtered =
      filter === "all"
        ? visibleConversations
        : filter === "system"
          ? visibleConversations.filter(
              (e) => e.kind === "system" || e.kind === "workflow",
            )
          : visibleConversations.filter((e) => e.kind === filter);
    return maxItems ? filtered.slice(0, maxItems) : filtered;
  }, [allConversations, filter, maxItems]);

  const handleOpen = useCallback(
    (entry: BuddyConversationEntry) => {
      void dispatch(openExistingBuddyChat(entry));
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
    <Surface
      className={classNames(styles.panel, className)}
      data-testid="buddy-recent-chats"
      animated="rise"
      radius="card"
      variant="glass"
    >
      <BuddySectionHeader
        icon={MessagesSquare}
        label={title ?? (compact ? "Recent activity" : "Conversations")}
        actions={
          <>
            {onViewAll && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                onClick={onViewAll}
              >
                View all
              </Button>
            )}
            {!compact && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                leftIcon={Plus}
                loading={isCreating}
                onClick={() => void handleNew()}
              >
                New Chat
              </Button>
            )}
          </>
        }
      />

      {showFilters && !compact && (
        <SegmentedControl
          aria-label="conversation filter"
          className={styles.filter}
          name="buddy-recent-chats-filter"
          size="sm"
          value={filter}
          onValueChange={(value) => setFilter(value as FilterKind)}
          options={FILTER_OPTIONS}
        />
      )}

      {isLoading && (
        <LoadingState label="Loading recent chats" variant="compact" />
      )}

      {!isLoading && conversations.length === 0 && (
        <div className={styles.empty}>
          <Icon icon={MessagesSquare} size="lg" tone="faint" />
          <Text size="1" color="gray">
            {filter === "all" ? "No conversations yet" : `No ${filter} entries`}
          </Text>
          {filter === "all" && (
            <Button
              type="button"
              size="sm"
              variant="primary"
              onClick={() => void handleNew()}
            >
              Start a conversation
            </Button>
          )}
        </div>
      )}

      {conversations.length > 0 && (
        <div className={classNames(styles.scrollList, "rf-stagger")}>
          {conversations.map((entry) => (
            <EntryRow
              key={`${entry.kind}-${entry.id}`}
              entry={entry}
              onClick={handleOpen}
            />
          ))}
        </div>
      )}
    </Surface>
  );
};
