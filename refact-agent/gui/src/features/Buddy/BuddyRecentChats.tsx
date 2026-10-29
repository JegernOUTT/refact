import React, { useCallback, useMemo } from "react";
import { MessagesSquare, Plus } from "lucide-react";
import { Button, SegmentedControl } from "../../components/ui";
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
import {
  BUDDY_TERMS,
  BUDDY_VERBS,
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddyRow,
  BuddyScrollRail,
  BuddySection,
  BuddyStatusBadge,
  BuddyTimestamp,
  useBuddyPersistentState,
  writeBuddyPersistentState,
} from "./ui";
import { conversationIcon } from "./buddyIcons";
import type { BuddyConversationEntry } from "./types";
import styles from "./BuddyRecentChats.module.css";
import { collapseConsecutive, countLabel } from "./buddyHistory";

type RecentChatsFilter = "background" | "setup" | "chat" | "all";

const FILTER_KEY = "buddy.activity.chatsFilter";
const VISIBLE_KEY = "buddy.activity.chatsVisible";
const STALE_EMPTY_PLACEHOLDER_MS = 24 * 60 * 60 * 1000;
const COMPACT_LOADING_ROWS = 3;
export const CHATS_PAGE_SIZE = 10;

const FILTERS: Record<
  RecentChatsFilter,
  { label: string; heading: string; empty: string }
> = {
  background: {
    label: "Background",
    heading: BUDDY_TERMS.backgroundChats,
    empty: "No background chats yet",
  },
  setup: {
    label: "Setup",
    heading: "Setup chats",
    empty: "No setup chats yet",
  },
  chat: {
    label: "Chats",
    heading: "Conversations",
    empty: "No conversations yet",
  },
  all: { label: "All", heading: "All chats", empty: "No chats yet" },
};

const FILTER_OPTIONS = (Object.keys(FILTERS) as RecentChatsFilter[]).map(
  (value) => ({ value, label: FILTERS[value].label }),
);

function isFilter(value: string): value is RecentChatsFilter {
  return value in FILTERS;
}

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

function matchesFilter(
  entry: BuddyConversationEntry,
  filter: RecentChatsFilter,
): boolean {
  if (filter === "all") return true;
  if (filter === "background")
    return entry.kind === "system" || entry.kind === "workflow";
  return entry.kind === filter;
}

function entryTitle(entry: BuddyConversationEntry): string {
  return isWorkflowHistory(entry) &&
    entry.badge &&
    entry.title === `${entry.id.replace(/_/g, " ")} (${entry.badge})`
    ? entry.badge
    : entry.title || "Untitled";
}

function collapseKey(entry: BuddyConversationEntry): string {
  if (!isWorkflowHistory(entry)) return `${entry.kind}\u0000${entry.id}`;
  return `${entry.badge ?? entry.id}\u0000${entry.title}`;
}

interface EntryRowProps {
  entry: BuddyConversationEntry;
  onOpen: (entry: BuddyConversationEntry) => void;
  duplicateCount: number;
  compact: boolean;
}

const EntryRow: React.FC<EntryRowProps> = ({
  entry,
  onOpen,
  duplicateCount,
  compact,
}) => {
  const workflowHistory = isWorkflowHistory(entry);
  const clickable =
    entry.kind === "chat" || entry.kind === "setup" || workflowHistory;
  const title = entryTitle(entry);
  const failed = workflowHistory && entry.status === "failed";
  const badge =
    entry.badge && !title.toLowerCase().includes(entry.badge.toLowerCase())
      ? entry.badge
      : null;
  return (
    <BuddyRow
      title={title}
      leading={{ icon: conversationIcon(entry.kind) }}
      status={
        failed ? <BuddyStatusBadge dimension="health" value="failed" /> : null
      }
      meta={[
        badge,
        duplicateCount > 1 ? countLabel(duplicateCount, "chat", "chats") : null,
        entry.message_count > 0
          ? workflowHistory
            ? countLabel(entry.message_count, "entry", "entries")
            : countLabel(entry.message_count, "message", "messages")
          : null,
        entry.updated_at ? <BuddyTimestamp value={entry.updated_at} /> : null,
      ]}
      density={compact ? "compact" : "regular"}
      onClick={clickable ? () => onOpen(entry) : undefined}
      aria-label={clickable ? `Open ${title}` : undefined}
      data-kind={entry.kind}
    />
  );
};

interface BuddyRecentChatsProps {
  compact?: boolean;
  maxItems?: number;
  showFilters?: boolean;
  onViewAll?: () => void;
  title?: string;
  className?: string;
  initialFilter?: RecentChatsFilter;
}

export const BuddyRecentChats: React.FC<BuddyRecentChatsProps> = ({
  compact = false,
  maxItems,
  showFilters = true,
  onViewAll,
  title,
  className,
  initialFilter,
}) => {
  const dispatch = useAppDispatch();
  const filterable = showFilters && !compact;
  const fixedFilter = filterable ? undefined : initialFilter ?? "all";
  const [storedFilter, setStoredFilter] =
    useBuddyPersistentState<RecentChatsFilter>(
      filterable ? FILTER_KEY : undefined,
      initialFilter ?? "background",
    );
  const filter = fixedFilter ?? storedFilter;
  const [visibleCount, setVisibleCount] = useBuddyPersistentState(
    compact ? undefined : VISIBLE_KEY,
    CHATS_PAGE_SIZE,
  );
  const {
    data: conversationsList,
    isLoading,
    isError,
    error,
    isFetching,
    refetch,
  } = useGetBuddyConversationsQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const allConversations = conversationsList?.entries;
  const [createConversation, { isLoading: isCreating }] =
    useCreateBuddyConversationMutation();

  const collapsed = useMemo(() => {
    const visible = (allConversations ?? []).filter(
      (entry) =>
        !isStaleEmptyPlaceholder(entry) && matchesFilter(entry, filter),
    );
    return collapseConsecutive(visible, collapseKey);
  }, [allConversations, filter]);
  const visibleConversations = collapsed.slice(0, maxItems ?? visibleCount);

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
  const handleViewAll = useCallback(() => {
    writeBuddyPersistentState<RecentChatsFilter>(FILTER_KEY, "chat");
    onViewAll?.();
  }, [onViewAll]);

  const copy = FILTERS[filter];
  const action = compact ? (
    onViewAll ? (
      <Button type="button" size="sm" variant="ghost" onClick={handleViewAll}>
        {BUDDY_VERBS.viewAll}
      </Button>
    ) : null
  ) : (
    <Button
      type="button"
      size="sm"
      variant="ghost"
      leftIcon={Plus}
      loading={isCreating}
      onClick={() => void handleNew()}
    >
      {BUDDY_VERBS.newChat}
    </Button>
  );

  return (
    <BuddySection
      title={
        title ?? (compact ? BUDDY_TERMS.recentConversations : copy.heading)
      }
      icon={MessagesSquare}
      count={compact || isLoading || isError ? undefined : collapsed.length}
      action={action}
      className={className}
      testId="buddy-recent-chats"
      footer={
        maxItems === undefined &&
        visibleConversations.length < collapsed.length ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => setVisibleCount((count) => count + CHATS_PAGE_SIZE)}
          >
            {BUDDY_VERBS.showMore}
          </Button>
        ) : null
      }
    >
      {filterable && (
        <BuddyScrollRail watch={filter}>
          <SegmentedControl
            aria-label="Chat filter"
            layout="content"
            name="buddy-recent-chats-filter"
            size="md"
            value={filter}
            onValueChange={(value) => {
              if (isFilter(value)) setStoredFilter(value);
            }}
            options={FILTER_OPTIONS}
          />
        </BuddyScrollRail>
      )}
      {isLoading ? (
        <BuddyLoading
          compact
          count={compact ? COMPACT_LOADING_ROWS : undefined}
          label="Loading chats"
        />
      ) : isError ? (
        <BuddyErrorCallout
          compact
          thing="chats"
          error={error}
          onRetry={() => void refetch()}
          retrying={isFetching}
        />
      ) : collapsed.length === 0 ? (
        <BuddyEmptyState compact icon={MessagesSquare} title={copy.empty} />
      ) : (
        <ul className={styles.list}>
          {visibleConversations.map(({ entry, count }) => (
            <li key={`${entry.kind}-${entry.id}`}>
              <EntryRow
                entry={entry}
                onOpen={handleOpen}
                duplicateCount={count}
                compact={compact}
              />
            </li>
          ))}
        </ul>
      )}
    </BuddySection>
  );
};
