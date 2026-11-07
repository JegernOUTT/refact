import { MessageSquare, RefreshCw } from "lucide-react";

import {
  Badge,
  type BadgeTone,
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
  StatusDot,
  type StatusDotStatus,
} from "../../../components/ui";
import {
  ACTIVE_CHAT_REASON_LABEL,
  activeChatsNeedAttention,
  type ActiveChatEntry,
  type ActiveChatReason,
} from "./activeChatsModel";
import styles from "./ActiveChatsSection.module.css";
import { type ActiveChatsStaleReason, useActiveChats } from "./useActiveChats";

const REASON_TONE: Record<ActiveChatReason, BadgeTone> = {
  streaming: "accent",
  working: "accent",
  recovering: "warning",
  needs_attention: "danger",
  scheduled: "default",
  schedule_paused: "muted",
};

const REASON_DOT: Record<ActiveChatReason, StatusDotStatus> = {
  streaming: "running",
  working: "running",
  recovering: "warning",
  needs_attention: "needs_attention",
  scheduled: "idle",
  schedule_paused: "paused",
};

export type ActiveChatsSectionViewProps = {
  entries: ActiveChatEntry[];
  isLoading: boolean;
  isError: boolean;
  isStale: boolean;
  staleReason?: ActiveChatsStaleReason;
  isOfflineWithoutData?: boolean;
  isFetching: boolean;
  onRetry: () => void;
  onOpenChat: (chatId: string) => void;
};

export function ActiveChatsSectionView({
  entries,
  isLoading,
  isError,
  isStale,
  staleReason = "refresh_failed",
  isOfflineWithoutData = false,
  isFetching,
  onRetry,
  onOpenChat,
}: ActiveChatsSectionViewProps) {
  const retry = (
    <Button
      disabled={isFetching}
      leftIcon={RefreshCw}
      size="sm"
      variant="soft"
      onClick={onRetry}
    >
      Retry
    </Button>
  );
  const hasAttention = activeChatsNeedAttention(entries);
  const hasRunning = entries.some((entry) =>
    entry.reasons.some(
      (reason) => reason === "streaming" || reason === "working",
    ),
  );

  return (
    <section aria-label="Active chats" className={styles.section}>
      <header className={styles.header}>
        <span className={styles.heading}>Active chats</span>
        {hasAttention ? (
          <StatusDot
            aria-label="Chats need attention"
            size="small"
            status="needs_attention"
          />
        ) : hasRunning ? (
          <StatusDot
            aria-label="Chats running"
            pulse
            size="small"
            status="running"
          />
        ) : null}
      </header>
      {isStale ? (
        <div className={styles.stale} role="status">
          <span>
            {staleReason === "offline"
              ? "Showing last known activity; engine is offline."
              : "Showing last known activity; refresh failed."}
          </span>
          {staleReason === "offline" ? null : retry}
        </div>
      ) : null}
      {isOfflineWithoutData ? (
        <EmptyState
          className={styles.state}
          description="Chat activity appears once the engine is reachable."
          icon={MessageSquare}
          title="Engine offline"
        />
      ) : isLoading ? (
        <LoadingState className={styles.state} label="Loading active chats" />
      ) : isError && !isStale ? (
        <ErrorState
          className={styles.state}
          description="Could not load chat activity for this project."
          retry={retry}
          title="Active chats unavailable"
        />
      ) : entries.length === 0 ? (
        <EmptyState
          className={styles.state}
          description="Streaming, recovering and scheduled chats appear here."
          icon={MessageSquare}
          title="No active chats"
        />
      ) : (
        <ul className={styles.list}>
          {entries.map((entry) => {
            const lead = entry.reasons[0];
            const labels = entry.reasons
              .map((reason) => ACTIVE_CHAT_REASON_LABEL[reason])
              .join(", ");
            return (
              <li key={entry.id}>
                <button
                  aria-label={`${entry.title} — ${labels}`}
                  className={`${styles.row} rf-pressable`}
                  data-testid="active-chat-row"
                  type="button"
                  onClick={() => onOpenChat(entry.id)}
                >
                  <StatusDot
                    aria-hidden="true"
                    className={styles.rowDot}
                    pulse={lead === "streaming" || lead === "working"}
                    size="small"
                    status={REASON_DOT[lead]}
                  />
                  <span className={styles.copy}>
                    <span className={styles.title}>{entry.title}</span>
                    <span className={styles.badges}>
                      {entry.reasons.map((reason) => (
                        <Badge
                          key={reason}
                          size="xs"
                          tone={REASON_TONE[reason]}
                          variant="outline"
                        >
                          {ACTIVE_CHAT_REASON_LABEL[reason]}
                        </Badge>
                      ))}
                    </span>
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

export function ActiveChatsSection({
  onOpenChat,
}: {
  onOpenChat: (chatId: string) => void;
}) {
  const activity = useActiveChats();
  return (
    <ActiveChatsSectionView
      entries={activity.entries}
      isError={activity.isError}
      isFetching={activity.isFetching}
      isLoading={activity.isLoading}
      isStale={activity.isStale}
      staleReason={activity.staleReason}
      isOfflineWithoutData={activity.isOfflineWithoutData}
      onOpenChat={onOpenChat}
      onRetry={activity.refetch}
    />
  );
}
