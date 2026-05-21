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
  selectActiveSpeech,
  selectSeenNotificationIds,
  dismissBuddySuggestion,
  dismissRuntimeEvent,
  clearActiveSpeech,
  markBuddyNotificationSeen,
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
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import {
  formatOpportunityActionError,
  useExecuteBuddyAction,
} from "./hooks/useExecuteBuddyAction";
import type {
  BuddyControl,
  BuddyOpportunity,
  BuddyRuntimeEvent,
  BuddySuggestion,
  DiagnosticContext,
} from "./types";
import { isBuddyOverlaySuppressedIssue } from "./investigation";
import { executeBuddyAction } from "./executeBuddyAction";
import {
  getOpportunityActionFromControl,
  getOpportunityActionIndexFromControl,
  getOpportunityDismissAction,
  opportunityActionControls,
  opportunitySpeechText,
} from "./buddyOpportunityActions";
import { isBuddyRuntimeEventVisible } from "./buddyRuntimeEvents";
import {
  compareBuddyRuntimeEvents,
  formatBuddyRuntimeEventText,
  isBuddySpeechExpired,
} from "./buddySceneSpeech";

import styles from "./BuddyChatCompanion.module.css";

interface Props {
  chatId: string;
}

interface NotificationItem {
  id: string;
  sourceId: string;
  text: string;
  source:
    | "speech"
    | "thread"
    | "runtime"
    | "diagnostic"
    | "suggestion"
    | "opportunity";
  controls: BuddyControl[];
  diagnostic?: DiagnosticContext | null;
  opportunity?: BuddyOpportunity;
  speechIntent?: string;
}

function notificationTriggerSource(
  source: NotificationItem["source"],
): "thread" | "runtime" | "diagnostic" | "suggestion" | "frontend" {
  if (source === "speech") return "runtime";
  if (source === "opportunity") return "suggestion";
  return source;
}

function notificationIdentity(
  source: NotificationItem["source"] | "thread-error",
  id: string,
): string {
  return `${source}:${id}`;
}

function createdAtMs(value: string): number {
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function speechMatchesChat(
  activeSpeech: { chat_id?: string } | null,
  chatId: string,
): boolean {
  return !activeSpeech?.chat_id || activeSpeech.chat_id === chatId;
}

function speechExpiryDelayMs(
  activeSpeech: {
    created_at: string;
    persistent: boolean;
    ttl_seconds: number;
  } | null,
): number | null {
  if (
    !activeSpeech ||
    activeSpeech.persistent ||
    activeSpeech.ttl_seconds <= 0
  ) {
    return null;
  }
  const createdAt = Date.parse(activeSpeech.created_at);
  if (!Number.isFinite(createdAt)) return null;
  return Math.max(
    0,
    createdAt + activeSpeech.ttl_seconds * 1000 - Date.now() + 1,
  );
}

function runtimeCandidates(
  chatId: string,
  nowPlaying: BuddyRuntimeEvent | null,
  runtimeQueue: BuddyRuntimeEvent[],
  chatDiagnostic: DiagnosticContext | null,
): BuddyRuntimeEvent[] {
  return [nowPlaying, ...runtimeQueue]
    .filter(
      (event): event is BuddyRuntimeEvent =>
        event?.chat_id === chatId &&
        isBuddyRuntimeEventVisible(event) &&
        !isBuddyOverlaySuppressedIssue(
          formatBuddyRuntimeEventText(event),
          chatDiagnostic,
        ),
    )
    .sort(compareBuddyRuntimeEvents);
}

export const BuddyChatCompanion: React.FC<Props> = ({ chatId }) => {
  const dispatch = useAppDispatch();
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const seenNotificationIds = useAppSelector(selectSeenNotificationIds);
  const threadError = useAppSelector((state) =>
    selectChatErrorById(state, chatId),
  );

  const buddy = useBuddyState();
  const { unread } = useBuddyOpportunities();
  const executeOpportunityAction = useExecuteBuddyAction();
  const [dismissMutation] = useDismissBuddySuggestionMutation();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();

  const [dismissedNotificationIds, setDismissedNotificationIds] = useState<
    Set<string>
  >(new Set());
  const [activeNotificationId, setActiveNotificationId] = useState<
    string | null
  >(null);
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [, refreshSpeechExpiry] = useState(0);
  const pendingRef = useRef(false);
  const prevChatIdRef = useRef(chatId);

  useEffect(() => {
    if (prevChatIdRef.current !== chatId) {
      prevChatIdRef.current = chatId;
      setDismissedNotificationIds(new Set());
      setActiveNotificationId(null);
      setActionError(null);
    }
  }, [chatId]);

  useEffect(() => {
    const delayMs = speechExpiryDelayMs(activeSpeech);
    if (delayMs == null) return;
    const timer = window.setTimeout(() => {
      refreshSpeechExpiry((tick) => tick + 1);
    }, delayMs);
    return () => window.clearTimeout(timer);
  }, [activeSpeech]);

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

  const dismissNotification = useCallback(
    (id: string) => {
      dispatch(markBuddyNotificationSeen(id));
      setDismissedNotificationIds((prev) => new Set(prev).add(id));
      setActiveNotificationId((current) => (current === id ? null : current));
    },
    [dispatch],
  );

  const restoreNotification = useCallback((id: string) => {
    setDismissedNotificationIds((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
    setActiveNotificationId(id);
  }, []);

  const notification = useMemo<NotificationItem | null>(() => {
    const isEligible = (id: string) =>
      !dismissedNotificationIds.has(id) &&
      (!(id in seenNotificationIds) || activeNotificationId === id);

    const chatDiagnostic =
      diagnostics.find((d) => d.chat_id === chatId) ?? null;

    if (
      activeSpeech &&
      !isBuddySpeechExpired(activeSpeech) &&
      speechMatchesChat(activeSpeech, chatId)
    ) {
      const id = notificationIdentity("speech", activeSpeech.id);
      if (isEligible(id)) {
        return {
          id,
          sourceId: activeSpeech.id,
          text: activeSpeech.text,
          source: "speech",
          controls: activeSpeech.controls,
          diagnostic: activeSpeech.chat_id
            ? (diagnostics.find((d) => d.chat_id === activeSpeech.chat_id) ??
              null)
            : null,
          speechIntent: activeSpeech.speech_intent,
        };
      }
    }

    const runtimes = runtimeCandidates(
      chatId,
      nowPlaying,
      runtimeQueue,
      chatDiagnostic,
    );
    const criticalRuntime = runtimes.find((event) => {
      const id = notificationIdentity("runtime", event.id);
      return event.priority === "critical" && isEligible(id);
    });
    if (criticalRuntime) {
      return {
        id: notificationIdentity("runtime", criticalRuntime.id),
        sourceId: criticalRuntime.id,
        text: formatBuddyRuntimeEventText(criticalRuntime),
        source: "runtime",
        controls: criticalRuntime.controls?.length
          ? criticalRuntime.controls
          : errorControls,
        diagnostic: chatDiagnostic,
      };
    }

    const normalizedThreadError = threadError?.trim() ?? null;
    const threadId = notificationIdentity("thread-error", chatId);
    if (normalizedThreadError && isEligible(threadId)) {
      if (
        isBuddyOverlaySuppressedIssue(normalizedThreadError, chatDiagnostic)
      ) {
        return null;
      }
      return {
        id: threadId,
        sourceId: chatId,
        text: normalizedThreadError.slice(0, 160),
        source: "thread",
        controls: errorControls,
        diagnostic: chatDiagnostic,
      };
    }

    const runtimeError = runtimes.find((event) =>
      isEligible(notificationIdentity("runtime", event.id)),
    );
    if (runtimeError) {
      return {
        id: notificationIdentity("runtime", runtimeError.id),
        sourceId: runtimeError.id,
        text: formatBuddyRuntimeEventText(runtimeError),
        source: "runtime",
        controls: runtimeError.controls?.length
          ? runtimeError.controls
          : errorControls,
        diagnostic: chatDiagnostic,
      };
    }

    if (chatDiagnostic?.error_message.trim()) {
      const id = notificationIdentity(
        "diagnostic",
        `${chatId}:${chatDiagnostic.collected_at}`,
      );
      if (isEligible(id)) {
        if (
          isBuddyOverlaySuppressedIssue(
            chatDiagnostic.error_message,
            chatDiagnostic,
          )
        ) {
          return null;
        }
        return {
          id,
          sourceId: chatDiagnostic.diagnostic_id ?? chatDiagnostic.collected_at,
          text: chatDiagnostic.error_message.slice(0, 120),
          source: "diagnostic",
          controls: errorControls,
          diagnostic: chatDiagnostic,
        };
      }
    }

    const activeSuggestion = suggestions.find((suggestion: BuddySuggestion) => {
      const id = notificationIdentity("suggestion", suggestion.id);
      return !suggestion.dismissed && isEligible(id);
    });
    if (activeSuggestion) {
      return {
        id: notificationIdentity("suggestion", activeSuggestion.id),
        sourceId: activeSuggestion.id,
        text: `${activeSuggestion.title}: ${activeSuggestion.description}`,
        source: "suggestion",
        controls: activeSuggestion.controls.length
          ? activeSuggestion.controls
          : suggestionControls,
        diagnostic: null,
      };
    }

    const activeOpportunity = unread
      .filter((opportunity) =>
        isEligible(notificationIdentity("opportunity", opportunity.id)),
      )
      .sort(
        (left, right) =>
          createdAtMs(right.created_at) - createdAtMs(left.created_at),
      )
      .at(0);
    return activeOpportunity === undefined
      ? null
      : {
          id: notificationIdentity("opportunity", activeOpportunity.id),
          sourceId: activeOpportunity.id,
          text: opportunitySpeechText(activeOpportunity),
          source: "opportunity",
          controls: opportunityActionControls(activeOpportunity),
          diagnostic: null,
          opportunity: activeOpportunity,
        };
  }, [
    activeNotificationId,
    activeSpeech,
    chatId,
    diagnostics,
    dismissedNotificationIds,
    errorControls,
    nowPlaying,
    runtimeQueue,
    seenNotificationIds,
    suggestionControls,
    suggestions,
    threadError,
    unread,
  ]);

  useEffect(() => {
    setActionError(null);
  }, [notification?.id]);

  useEffect(() => {
    if (!notification) {
      setActiveNotificationId(null);
      return;
    }
    if (activeNotificationId === notification.id) return;
    setActiveNotificationId(notification.id);
  }, [activeNotificationId, notification]);

  useEffect(() => {
    if (!activeNotificationId) return;
    if (activeNotificationId in seenNotificationIds) return;
    dispatch(markBuddyNotificationSeen(activeNotificationId));
  }, [activeNotificationId, dispatch, seenNotificationIds]);

  const handleControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (!notification) return;

      if (notification.source === "opportunity") {
        if (pendingRef.current || !notification.opportunity) return;
        const actionIndex = getOpportunityActionIndexFromControl(ctrl);
        if (actionIndex == null) return;
        const action = getOpportunityActionFromControl(
          ctrl,
          notification.opportunity,
        );
        if (!action) return;

        pendingRef.current = true;
        setPending(true);
        setActionError(null);
        try {
          if (action.kind === "dismiss") {
            const results = await Promise.allSettled(
              [notification.opportunity].map(async (opp) => {
                const dismissAction = getOpportunityDismissAction(opp);
                await executeOpportunityAction(
                  dismissAction.action,
                  opp,
                  dismissAction.actionIndex,
                );
                return opp.id;
              }),
            );
            const dismissedOpportunityIds = results.flatMap((result) =>
              result.status === "fulfilled" ? [result.value] : [],
            );
            if (dismissedOpportunityIds.length > 0) {
              for (const oppId of dismissedOpportunityIds) {
                dismissNotification(notificationIdentity("opportunity", oppId));
              }
            }
            const failed = results.find(
              (result) => result.status === "rejected",
            );
            if (failed) {
              restoreNotification(notification.id);
              setActionError(formatOpportunityActionError(failed.reason));
            }
            return;
          }

          await executeOpportunityAction(
            action,
            notification.opportunity,
            actionIndex,
          );
          dismissNotification(notification.id);
        } catch (error) {
          restoreNotification(notification.id);
          setActionError(formatOpportunityActionError(error));
        } finally {
          pendingRef.current = false;
          setPending(false);
        }
        return;
      }

      if (ctrl.action === "dismiss" || ctrl.action === "dismiss_speech") {
        dismissNotification(notification.id);
        setActionError(null);
        if (notification.source === "speech") {
          dispatch(clearActiveSpeech());
        } else if (notification.source === "suggestion") {
          try {
            await dismissMutation(notification.sourceId).unwrap();
            dispatch(dismissBuddySuggestion(notification.sourceId));
          } catch (error) {
            restoreNotification(notification.id);
            setActionError(formatOpportunityActionError(error));
          }
        } else if (notification.source === "runtime") {
          dispatch(dismissRuntimeEvent(notification.sourceId));
          void dismissRuntimeMutation(notification.sourceId)
            .unwrap()
            .catch(() => undefined);
        }
        return;
      }

      if (ctrl.action === "dismiss_runtime_event") {
        const runtimeEventId = ctrl.action_param?.trim()
          ? ctrl.action_param.trim()
          : notification.sourceId;
        const runtimeNotificationId = notificationIdentity(
          "runtime",
          runtimeEventId,
        );
        dismissNotification(notification.id);
        setActionError(null);
        if (notification.id !== runtimeNotificationId) {
          dismissNotification(runtimeNotificationId);
        }
        dispatch(dismissRuntimeEvent(runtimeEventId));
        void dismissRuntimeMutation(runtimeEventId)
          .unwrap()
          .catch(() => undefined);
        return;
      }

      if (ctrl.action === "open_buddy") {
        dismissNotification(notification.id);
        dispatch(push({ name: "buddy" }));
        return;
      }

      if (ctrl.action.startsWith("care_")) {
        await executeBuddyAction(ctrl, dispatch);
        dismissNotification(notification.id);
        return;
      }

      if (ctrl.action === "accept_quest") {
        await executeBuddyAction(ctrl, dispatch, {
          triggerText: notification.text,
          triggerSource: notificationTriggerSource(notification.source),
          sourceChatId: chatId,
          diagnostic: notification.diagnostic,
        });
        if (notification.source === "suggestion") {
          dispatch(dismissBuddySuggestion(notification.sourceId));
        }
        dismissNotification(notification.id);
        return;
      }

      if (ctrl.action === "investigate_error") {
        if (pendingRef.current || pending) return;
        pendingRef.current = true;
        setPending(true);
        setActionError(null);
        try {
          if (notification.source === "suggestion") {
            dismissNotification(notification.id);
            await dismissMutation(notification.sourceId).unwrap();
            dispatch(dismissBuddySuggestion(notification.sourceId));
          } else if (notification.source === "runtime") {
            dispatch(dismissRuntimeEvent(notification.sourceId));
            void dismissRuntimeMutation(notification.sourceId)
              .unwrap()
              .catch(() => undefined);
          }
          await dispatch(
            startBuddyInvestigation({
              triggerText: notification.text,
              triggerSource: notificationTriggerSource(notification.source),
              sourceChatId: chatId,
              diagnostic: notification.diagnostic,
            }),
          ).unwrap();
          if (notification.source !== "suggestion") {
            dismissNotification(notification.id);
          }
        } catch (error) {
          if (notification.source === "suggestion") {
            restoreNotification(notification.id);
          }
          setActionError(formatOpportunityActionError(error));
        } finally {
          pendingRef.current = false;
          setPending(false);
        }
      }
    },
    [
      notification,
      pending,
      executeOpportunityAction,
      dismissMutation,
      dismissRuntimeMutation,
      dismissNotification,
      restoreNotification,
      dispatch,
      chatId,
    ],
  );

  if (!enabled) return null;
  if (!notification) return null;

  return (
    <div className={styles.companion} data-notification-id={notification.id}>
      <BuddyCanvas
        state={buddy.state}
        onEvent={buddy.handleCanvasEvent}
        displaySize={160}
        speechOverride={actionError ?? notification.text}
        speechControls={notification.controls}
        speechIntent={notification.speechIntent}
        onSpeechControlClick={(ctrl) => void handleControl(ctrl)}
        bubblePosition="left"
      />
    </div>
  );
};
