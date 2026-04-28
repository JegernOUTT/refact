import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectNowPlaying,
  selectBuddyDiagnostics,
  selectIsBuddyEnabled,
  selectRuntimeQueue,
  selectBuddySuggestions,
  dismissBuddySuggestion,
  dismissRuntimeEvent,
} from "./buddySlice";
import { selectChatErrorById } from "../Chat/Thread";
import { startBuddyInvestigation } from "../Chat/Thread";
import { push } from "../Pages/pagesSlice";
import {
  useDismissBuddySuggestionMutation,
  useDismissBuddyRuntimeEventMutation,
} from "../../services/refact/buddy";
import { useBuddyState } from "./hooks/useBuddyState";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddyOpportunityCard } from "./BuddyOpportunityCard";
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import type { BuddyControl, BuddySuggestion, DiagnosticContext } from "./types";
import { isBuddyOverlaySuppressedIssue } from "./investigation";
import { executeBuddyAction } from "./executeBuddyAction";
import { selectBuddySnapshot } from "./buddySlice";
import styles from "./BuddyChatCompanion.module.css";

interface Props {
  chatId: string;
}

interface NotificationItem {
  id: string;
  text: string;
  source: "thread" | "runtime" | "diagnostic" | "suggestion";
  controls: BuddyControl[];
  timestamp: number;
  diagnostic?: DiagnosticContext | null;
}

export const BuddyChatCompanion: React.FC<Props> = ({ chatId }) => {
  const dispatch = useAppDispatch();
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const snapshot = useAppSelector(selectBuddySnapshot);
  const threadError = useAppSelector((state) =>
    selectChatErrorById(state, chatId),
  );

  const buddy = useBuddyState();
  const { unread } = useBuddyOpportunities();
  const [showTopOpp, setShowTopOpp] = useState(false);
  const [dismissMutation] = useDismissBuddySuggestionMutation();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();

  const [dismissedIds, setDismissedIds] = useState<Set<string>>(new Set());
  const [pending, setPending] = useState(false);
  const prevChatIdRef = useRef(chatId);

  useEffect(() => {
    if (prevChatIdRef.current !== chatId) {
      prevChatIdRef.current = chatId;
      setDismissedIds(new Set());
    }
  }, [chatId]);

  const errorControls: BuddyControl[] = useMemo(
    () => [
      {
        id: "ask",
        label: "Investigate",
        action: "investigate_error",
        style: "primary",
      },
      {
        id: "dismiss",
        label: "Dismiss",
        action: "dismiss",
        style: "ghost",
      },
    ],
    [],
  );

  const suggestionControls: BuddyControl[] = useMemo(
    () => [
      {
        id: "fix",
        label: "Investigate",
        action: "investigate_error",
        style: "primary",
      },
      {
        id: "ignore",
        label: "Ignore",
        action: "dismiss",
        style: "ghost",
      },
    ],
    [],
  );

  const notification: NotificationItem | null = useMemo(() => {
    const chatDiagnostic =
      diagnostics.find((d) => d.chat_id === chatId) ?? null;
    const normalizedThreadError = threadError?.trim() || null;
    if (normalizedThreadError) {
      if (
        isBuddyOverlaySuppressedIssue(normalizedThreadError, chatDiagnostic)
      ) {
        return null;
      }
      return {
        id: `thread-${chatId}`,
        text: normalizedThreadError.slice(0, 160),
        source: "thread",
        controls: errorControls,
        timestamp: Date.now(),
        diagnostic: chatDiagnostic,
      };
    }

    const runtimeError =
      nowPlaying?.chat_id === chatId &&
      nowPlaying?.status === "failed" &&
      !nowPlaying?.dismissed
        ? nowPlaying
        : runtimeQueue.find(
            (e) =>
              e.chat_id === chatId && e.status === "failed" && !e.dismissed,
          ) ?? null;
    if (runtimeError) {
      if (isBuddyOverlaySuppressedIssue(runtimeError.title, chatDiagnostic)) {
        return null;
      }
      return {
        id: runtimeError.id,
        text: runtimeError.title,
        source: "runtime",
        controls: runtimeError.controls?.length
          ? runtimeError.controls
          : errorControls,
        timestamp: new Date(runtimeError.created_at).getTime(),
        diagnostic: chatDiagnostic,
      };
    }

    if (chatDiagnostic?.error_message?.trim()) {
      if (
        isBuddyOverlaySuppressedIssue(
          chatDiagnostic.error_message,
          chatDiagnostic,
        )
      ) {
        return null;
      }
      return {
        id: `diag-${chatId}-${chatDiagnostic.collected_at}`,
        text: chatDiagnostic.error_message.slice(0, 120),
        source: "diagnostic",
        controls: errorControls,
        timestamp: new Date(chatDiagnostic.collected_at).getTime(),
        diagnostic: chatDiagnostic,
      };
    }

    const activeSuggestion = suggestions.find(
      (s: BuddySuggestion) => !s.dismissed,
    );
    if (activeSuggestion) {
      return {
        id: activeSuggestion.id,
        text: `${activeSuggestion.title}: ${activeSuggestion.description}`,
        source: "suggestion",
        controls: activeSuggestion.controls?.length
          ? activeSuggestion.controls
          : suggestionControls,
        timestamp: new Date(activeSuggestion.created_at).getTime(),
        diagnostic: null,
      };
    }

    return null;
  }, [
    threadError,
    chatId,
    nowPlaying,
    runtimeQueue,
    diagnostics,
    suggestions,
    snapshot,
    errorControls,
    suggestionControls,
  ]);

  const isDismissed = notification ? dismissedIds.has(notification.id) : false;

  useEffect(() => {
    if (!notification || isDismissed) return;
    const t = setTimeout(() => {
      setDismissedIds((prev) => new Set(prev).add(notification.id));
    }, 15000);
    return () => clearTimeout(t);
  }, [notification, isDismissed]);

  const handleControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (!notification) return;

      if (ctrl.action === "dismiss" || ctrl.action === "dismiss_speech") {
        if (notification.source === "suggestion") {
          await dismissMutation(notification.id);
          dispatch(dismissBuddySuggestion(notification.id));
        } else if (notification.source === "runtime") {
          // Optimistically mark dismissed so the bubble disappears immediately,
          // then persist to the backend so it stays dismissed across reloads/SSE.
          dispatch(dismissRuntimeEvent(notification.id));
          try {
            await dismissRuntimeMutation(notification.id).unwrap();
          } catch {
            // Server unavailable: local-state fallback below still hides it
            // for this session.
          }
        }
        setDismissedIds((prev) => new Set(prev).add(notification.id));
        return;
      }

      if (ctrl.action === "open_buddy") {
        setDismissedIds((prev) => new Set(prev).add(notification.id));
        dispatch(push({ name: "buddy" }));
        return;
      }

      if (ctrl.action.startsWith("care_")) {
        await executeBuddyAction(ctrl, dispatch);
        setDismissedIds((prev) => new Set(prev).add(notification.id));
        return;
      }

      if (ctrl.action === "accept_quest") {
        await executeBuddyAction(ctrl, dispatch, {
          triggerText: notification.text,
          triggerSource: notification.source,
          sourceChatId: chatId,
          diagnostic: notification.diagnostic,
        });
        if (notification.source === "suggestion") {
          dispatch(dismissBuddySuggestion(notification.id));
        }
        setDismissedIds((prev) => new Set(prev).add(notification.id));
        return;
      }

      if (ctrl.action === "investigate_error") {
        if (pending) return;
        setPending(true);
        try {
          if (notification.source === "suggestion") {
            await dismissMutation(notification.id);
            dispatch(dismissBuddySuggestion(notification.id));
          } else if (notification.source === "runtime") {
            // Investigating an error implicitly resolves it — persist
            // dismissal so the bubble doesn't reappear after the
            // investigation chat opens.
            dispatch(dismissRuntimeEvent(notification.id));
            try {
              await dismissRuntimeMutation(notification.id).unwrap();
            } catch {
              // Non-fatal: local dismiss still hides it for this session.
            }
          }
          await dispatch(
            startBuddyInvestigation({
              triggerText: notification.text,
              triggerSource: notification.source,
              sourceChatId: chatId,
              diagnostic: notification.diagnostic,
            }),
          );
          setDismissedIds((prev) => new Set(prev).add(notification.id));
        } finally {
          setPending(false);
        }
      }
    },
    [
      notification,
      pending,
      dismissMutation,
      dismissRuntimeMutation,
      dispatch,
      chatId,
    ],
  );

  const badgeCount = unread.length;
  const badgeLabel = badgeCount > 9 ? "9+" : String(badgeCount);

  if (!enabled) return null;
  if (!notification || isDismissed) {
    if (badgeCount === 0) return null;
    return (
      <div className={styles.companion}>
        {badgeCount > 0 && (
          <button
            type="button"
            data-testid="buddy-companion-unread-badge"
            aria-label={`${badgeCount} unread opportunities`}
            style={{
              background: "var(--accent-9)",
              color: "var(--accent-contrast)",
              border: "none",
              borderRadius: "9999px",
              padding: "1px 5px",
              fontSize: "10px",
              fontWeight: 700,
              cursor: "pointer",
              lineHeight: 1.4,
            }}
            onClick={() => setShowTopOpp((v) => !v)}
          >
            {badgeLabel}
          </button>
        )}
        {showTopOpp && unread.length > 0 && (
          <BuddyOpportunityCard opportunity={unread[0]} />
        )}
      </div>
    );
  }

  return (
    <div className={styles.companion}>
      {badgeCount > 0 && (
        <button
          type="button"
          data-testid="buddy-companion-unread-badge"
          aria-label={`${badgeCount} unread opportunities`}
          style={{
            background: "var(--accent-9)",
            color: "var(--accent-contrast)",
            border: "none",
            borderRadius: "9999px",
            padding: "1px 5px",
            fontSize: "10px",
            fontWeight: 700,
            cursor: "pointer",
            lineHeight: 1.4,
          }}
          onClick={() => setShowTopOpp((v) => !v)}
        >
          {badgeLabel}
        </button>
      )}
      {showTopOpp && unread.length > 0 && (
        <BuddyOpportunityCard opportunity={unread[0]} />
      )}
      <BuddyCanvas
        state={buddy.state}
        onEvent={buddy.handleCanvasEvent}
        displaySize={160}
        speechOverride={notification.text}
        speechControls={notification.controls}
        onSpeechControlClick={handleControl}
        bubblePosition="left"
      />
    </div>
  );
};
