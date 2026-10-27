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
import { formatRelativeTime } from "./buddyUtils";
import type { BuddyConversationEntry } from "./types";
import styles from "./BuddyRecentChats.module.css";
import {
  collapseConsecutive,
  countLabel,
  HISTORY_PAGE_SIZE,
} from "./buddyHistory";

type FilterKind = "all" | "chat" | "setup" | "system";

const FILTER_OPTIONS: { value: FilterKind; label: string }[] = [
  { value: "all", label: "All" },
  { value: "chat", label: "Chats" },
  { value: "setup", label: "Setup" },
  { value: "system", label: "System" },
];

const STALE_EMPTY_PLACEHOLDER_MS = 24 * 60 * 60 * 1000;

function isWorkflowHistory(entry: BuddyConversationEntry): boolean {
  return (
    entry.kind === "workflow" ||
    (entry.kind === "system" &&
      entry.badge !== null &&
      entry.status !== "active")
  );
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
  duplicateCount: number;
}

const EntryRow: React.FC<EntryRowProps> = ({
  entry,
  onClick,
  duplicateCount,
}) => {
  const workflowHistory = isWorkflowHistory(entry);
  const clickable =
    entry.kind === "chat" || entry.kind === "setup" || workflowHistory;
  const title =
    workflowHistory &&
    entry.badge &&
    entry.title === `${entry.id.replace(/_/g, " ")} (${entry.badge})`
      ? entry.badge
      : entry.title || "Untitled";
  const content = (
    <>
      <span className={styles.entryIcon}>
        <Icon icon={conversationIcon(entry.kind)} size="sm" tone="muted" />
      </span>
      <span className={styles.entryBody}>
        <span className={styles.entryTitleRow}>
          <Text
            size="1"
            weight="medium"
            className={styles.entryTitle}
            title={title}
          >
            {title}
          </Text>
          {duplicateCount > 1 && (
            <Badge size="xs" tone="muted" className={styles.entryBadge}>
              {countLabel(duplicateCount, "chat", "chats")}
            </Badge>
          )}
          {entry.badge &&
            !title.toLowerCase().includes(entry.badge.toLowerCase()) && (
              <Badge size="xs" tone="muted" className={styles.entryBadge}>
                {entry.badge}
              </Badge>
            )}
        </span>
        <Text size="1" color="gray" className={styles.entryMeta}>
          {workflowHistory && entry.status === "failed" && (
            <>
              <Text size="1" color="red">
                Failed
              </Text>
              {entry.message_count > 0 ? " · " : ""}
            </>
          )}
          {entry.message_count > 0
            ? workflowHistory
              ? countLabel(entry.message_count, "entry", "entries")
              : countLabel(entry.message_count, "message", "messages")
            : workflowHistory && entry.status === "failed"
              ? null
              : entry.status}
          {entry.updated_at ? ` · ${formatRelativeTime(entry.updated_at)}` : ""}
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
      aria-label={`Open ${title}`}
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
  initialFilter?: FilterKind;
}

export const BuddyRecentChats: React.FC<BuddyRecentChatsProps> = ({
  compact = false,
  maxItems,
  showFilters = true,
  onViewAll,
  title,
  className,
  initialFilter = "all",
}) => {
  const dispatch = useAppDispatch();
  const [filter, setFilter] = useState<FilterKind>(initialFilter);
  const [visibleCount, setVisibleCount] = useState(HISTORY_PAGE_SIZE);
  const { data: conversationsList, isLoading } = useGetBuddyConversationsQuery(
    undefined,
    { refetchOnMountOrArgChange: true },
  );
  const allConversations = conversationsList?.entries;
  const [createConversation, { isLoading: isCreating }] =
    useCreateBuddyConversationMutation();

  const conversations = React.useMemo(() => {
    if (!allConversations) return [];
    const visible = allConversations.filter(
      (entry) => !isStaleEmptyPlaceholder(entry),
    );
    return filter === "all"
      ? visible
      : filter === "system"
        ? visible.filter(
            (entry) => entry.kind === "system" || entry.kind === "workflow",
          )
        : visible.filter((entry) => entry.kind === filter);
  }, [allConversations, filter]);
  const collapsedConversations = React.useMemo(
    () =>
      collapseConsecutive(conversations, (entry) => {
        if (!isWorkflowHistory(entry)) return `${entry.kind}\u0000${entry.id}`;
        return `${entry.badge ?? entry.id}\u0000${entry.title}`;
      }),
    [conversations],
  );
  const visibleConversations = collapsedConversations.slice(
    0,
    maxItems ?? visibleCount,
  );
  const handleOpen = useCallback(
    (entry: BuddyConversationEntry) => {
      void dispatch(
        openExistingBuddyChat(
          isWorkflowHistory(entry) && entry.kind === "system"
            ? { ...entry, kind: "workflow" }
            : entry,
        ),
      );
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
      {visibleConversations.length > 0 && (
        <ul className={classNames(styles.scrollList, "rf-stagger")}>
          {visibleConversations.map(({ entry, count }) => (
            <li key={`${entry.kind}-${entry.id}`}>
              <EntryRow
                entry={entry}
                onClick={handleOpen}
                duplicateCount={count}
              />
            </li>
          ))}
        </ul>
      )}
      {maxItems === undefined &&
        visibleConversations.length < collapsedConversations.length && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() =>
              setVisibleCount((count) => count + HISTORY_PAGE_SIZE)
            }
          >
            Show more
          </Button>
        )}
    </Surface>
  );
};
