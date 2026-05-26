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
  selectChatBubbleSnoozedUntil,
  selectChatBubbleImpressions,
  dismissBuddySuggestion,
  dismissRuntimeEvent,
  clearActiveSpeech,
  markBuddyNotificationSeen,
  recordChatBubbleImpression,
  snoozeChatBubbles,
  clearExpiredChatBubbleSnooze,
  type BuddyChatBubbleClass,
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
  createdAt: string;
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
  ttlMs?: number | null;
  ttlSeconds?: number;
}

interface NotificationCandidate {
  notification: NotificationItem;
  kind: BuddyChatBubbleClass;
  rank: number;
}

const EVENT_ONCE_FRESHNESS_MS = 75_000;
const AMBIENT_RATIO_TARGET = 0.5;
const AMBIENT_SIGNALS = new Set<string>([
  "speech_humor",
  "speech_insight",
  "speech_memory_pulse_commentary",
  "speaker_insight",
  "speaker_memory_pulse_commentary",
]);
const AMBIENT_INTENTS = new Set<string>([
  "humor",
  "insight",
  "memory_pulse_commentary",
]);
const DURABLE_SPEECH_INTENTS = new Set<string>([
  "tour",
  "quest_accept",
  "quest_complete",
  "milestone",
]);

function normalizedPolicyToken(value: string | null | undefined): string {
  return value?.trim().toLowerCase() ?? "";
}

function isAmbientToken(value: string | null | undefined): boolean {
  const token = normalizedPolicyToken(value);
  if (!token) return false;
  return AMBIENT_INTENTS.has(token) || AMBIENT_SIGNALS.has(token);
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

function eventFreshnessMs(ttlMs: number | null | undefined): number {
  if (ttlMs != null && Number.isFinite(ttlMs) && ttlMs > 0) {
    return Math.min(EVENT_ONCE_FRESHNESS_MS, ttlMs);
  }
  return EVENT_ONCE_FRESHNESS_MS;
}

function isFreshEventOnce(createdAt: string, ttlMs?: number | null): boolean {
  const createdAtTime = createdAtMs(createdAt);
  if (createdAtTime <= 0) return false;
  return Date.now() - createdAtTime <= eventFreshnessMs(ttlMs);
}

function isDurableSpeech(activeSpeech: {
  persistent: boolean;
  ttl_seconds: number;
  speech_intent?: string;
}): boolean {
  if (activeSpeech.persistent) return true;
  const intent = normalizedPolicyToken(activeSpeech.speech_intent);
  return intent ? DURABLE_SPEECH_INTENTS.has(intent) : false;
}

function classifySpeech(activeSpeech: {
  persistent: boolean;
  ttl_seconds: number;
  speech_intent?: string;
  dedupe_key?: string;
}): BuddyChatBubbleClass {
  if (
    isAmbientToken(activeSpeech.speech_intent) ||
    isAmbientToken(activeSpeech.dedupe_key)
  ) {
    return "ambient";
  }
  return isDurableSpeech(activeSpeech) ? "actionable" : "event_once";
}

function classifyRuntimeEvent(event: BuddyRuntimeEvent): BuddyChatBubbleClass {
  if (
    isAmbientToken(event.signal_type) ||
    isAmbientToken(event.source) ||
    isAmbientToken(event.dedupe_key ?? undefined)
  ) {
    return "ambient";
  }
  if (
    event.status === "failed" ||
    event.priority === "critical" ||
    event.priority === "high" ||
    (event.controls?.length ?? 0) > 0 ||
    event.persistent === true
  ) {
    return "actionable";
  }
  return "event_once";
}

function isCandidateFresh(candidate: NotificationCandidate): boolean {
  if (candidate.kind !== "event_once") return true;
  if (candidate.notification.source === "runtime") {
    return isFreshEventOnce(
      candidate.notification.createdAt,
      candidate.notification.ttlMs,
    );
  }
  if (candidate.notification.source === "speech") {
    const ttlMs = (candidate.notification.ttlSeconds ?? 0) * 1000;
    return isFreshEventOnce(candidate.notification.createdAt, ttlMs);
  }
  return true;
}

function ambientRatio(impressions: { kind: BuddyChatBubbleClass }[]): number {
  if (impressions.length === 0) return 0;
  const ambientCount = impressions.filter(
    (impression) => impression.kind === "ambient",
  ).length;
  return ambientCount / impressions.length;
}

function pickNotificationCandidate(
  candidates: NotificationCandidate[],
  impressions: { kind: BuddyChatBubbleClass }[],
): NotificationCandidate | null {
  const eligible = candidates.filter(isCandidateFresh);
  if (eligible.length === 0) return null;
  const sorted = [...eligible].sort((left, right) => left.rank - right.rank);
  const ambient = sorted.find((candidate) => candidate.kind === "ambient");
  if (ambient && ambientRatio(impressions) < AMBIENT_RATIO_TARGET) {
    return ambient;
  }
  return sorted[0] ?? null;
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
  const chatBubbleSnoozedUntil = useAppSelector(selectChatBubbleSnoozedUntil);
  const chatBubbleImpressions = useAppSelector(selectChatBubbleImpressions);
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

  const notificationCandidates = useMemo<NotificationCandidate[]>(() => {
    const isEligible = (id: string) =>
      !dismissedNotificationIds.has(id) &&
      (!(id in seenNotificationIds) || activeNotificationId === id);

    const chatDiagnostic =
      diagnostics.find((d) => d.chat_id === chatId) ?? null;
    const candidates: NotificationCandidate[] = [];

    if (
      activeSpeech &&
      !isBuddySpeechExpired(activeSpeech) &&
      speechMatchesChat(activeSpeech, chatId)
    ) {
      const id = notificationIdentity("speech", activeSpeech.id);
      if (isEligible(id)) {
        candidates.push({
          kind: classifySpeech(activeSpeech),
          rank: 10,
          notification: {
            id,
            sourceId: activeSpeech.id,
            text: activeSpeech.text,
            createdAt: activeSpeech.created_at,
            source: "speech",
            controls: activeSpeech.controls,
            diagnostic: activeSpeech.chat_id
              ? diagnostics.find((d) => d.chat_id === activeSpeech.chat_id) ??
                null
              : null,
            speechIntent: activeSpeech.speech_intent,
            ttlSeconds: activeSpeech.ttl_seconds,
          },
        });
      }
    }

    const runtimes = runtimeCandidates(
      chatId,
      nowPlaying,
      runtimeQueue,
      chatDiagnostic,
    );
    for (const [index, event] of runtimes.entries()) {
      const id = notificationIdentity("runtime", event.id);
      if (!isEligible(id)) continue;
      candidates.push({
        kind: classifyRuntimeEvent(event),
        rank: event.priority === "critical" ? 20 + index : 40 + index,
        notification: {
          id,
          sourceId: event.id,
          text: formatBuddyRuntimeEventText(event),
          createdAt: event.created_at,
          source: "runtime",
          controls: event.controls?.length ? event.controls : errorControls,
          diagnostic: chatDiagnostic,
          ttlMs: event.ttl_ms,
        },
      });
    }

    const normalizedThreadError = threadError?.trim() ?? null;
    const threadId = notificationIdentity("thread-error", chatId);
    if (
      normalizedThreadError &&
      isEligible(threadId) &&
      !isBuddyOverlaySuppressedIssue(normalizedThreadError, chatDiagnostic)
    ) {
      candidates.push({
        kind: "actionable",
        rank: 30,
        notification: {
          id: threadId,
          sourceId: chatId,
          text: normalizedThreadError.slice(0, 160),
          createdAt: new Date().toISOString(),
          source: "thread",
          controls: errorControls,
          diagnostic: chatDiagnostic,
        },
      });
    }

    if (chatDiagnostic?.error_message.trim()) {
      const id = notificationIdentity(
        "diagnostic",
        `${chatId}:${chatDiagnostic.collected_at}`,
      );
      if (
        isEligible(id) &&
        !isBuddyOverlaySuppressedIssue(
          chatDiagnostic.error_message,
          chatDiagnostic,
        )
      ) {
        candidates.push({
          kind: "actionable",
          rank: 50,
          notification: {
            id,
            sourceId:
              chatDiagnostic.diagnostic_id ?? chatDiagnostic.collected_at,
            text: chatDiagnostic.error_message.slice(0, 120),
            createdAt: chatDiagnostic.collected_at,
            source: "diagnostic",
            controls: errorControls,
            diagnostic: chatDiagnostic,
          },
        });
      }
    }

    suggestions.forEach((suggestion: BuddySuggestion, index) => {
      const id = notificationIdentity("suggestion", suggestion.id);
      if (suggestion.dismissed || !isEligible(id)) return;
      candidates.push({
        kind: "actionable",
        rank: 60 + index,
        notification: {
          id,
          sourceId: suggestion.id,
          text: `${suggestion.title}: ${suggestion.description}`,
          createdAt: suggestion.created_at,
          source: "suggestion",
          controls: suggestion.controls.length
            ? suggestion.controls
            : suggestionControls,
          diagnostic: null,
        },
      });
    });

    [...unread]
      .filter((opportunity) =>
        isEligible(notificationIdentity("opportunity", opportunity.id)),
      )
      .sort(
        (left, right) =>
          createdAtMs(right.created_at) - createdAtMs(left.created_at),
      )
      .forEach((opportunity, index) => {
        candidates.push({
          kind: "actionable",
          rank: 70 + index,
          notification: {
            id: notificationIdentity("opportunity", opportunity.id),
            sourceId: opportunity.id,
            text: opportunitySpeechText(opportunity),
            createdAt: opportunity.created_at,
            source: "opportunity",
            controls: opportunityActionControls(opportunity),
            diagnostic: null,
            opportunity,
          },
        });
      });

    return candidates;
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
    dispatch(clearExpiredChatBubbleSnooze());
    if (chatBubbleSnoozedUntil == null) return;
    const delayMs = Math.max(0, chatBubbleSnoozedUntil - Date.now() + 1);
    const timer = window.setTimeout(() => {
      dispatch(clearExpiredChatBubbleSnooze());
    }, delayMs);
    return () => window.clearTimeout(timer);
  }, [chatBubbleSnoozedUntil, dispatch]);

  useEffect(() => {
    const delays = notificationCandidates
      .filter((candidate) => candidate.kind === "event_once")
      .map((candidate) => {
        const ttlMs =
          candidate.notification.source === "speech"
            ? (candidate.notification.ttlSeconds ?? 0) * 1000
            : candidate.notification.ttlMs;
        return (
          createdAtMs(candidate.notification.createdAt) +
          eventFreshnessMs(ttlMs) -
          Date.now() +
          1
        );
      })
      .filter((delayMs) => delayMs > 0);
    if (delays.length === 0) return;
    const timer = window.setTimeout(
      () => {
        refreshSpeechExpiry((tick) => tick + 1);
      },
      Math.min(...delays),
    );
    return () => window.clearTimeout(timer);
  }, [notificationCandidates]);

  const selectedCandidate = useMemo<NotificationCandidate | null>(() => {
    if (chatBubbleSnoozedUntil != null && chatBubbleSnoozedUntil > Date.now()) {
      return null;
    }
    const activeCandidate = notificationCandidates.find(
      (candidate) =>
        candidate.notification.id === activeNotificationId &&
        isCandidateFresh(candidate),
    );
    if (activeCandidate) return activeCandidate;
    return pickNotificationCandidate(
      notificationCandidates,
      chatBubbleImpressions,
    );
  }, [
    activeNotificationId,
    chatBubbleImpressions,
    chatBubbleSnoozedUntil,
    notificationCandidates,
  ]);

  const notification = selectedCandidate?.notification ?? null;
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

  useEffect(() => {
    if (!selectedCandidate) return;
    dispatch(
      recordChatBubbleImpression({
        id: selectedCandidate.notification.id,
        kind: selectedCandidate.kind,
      }),
    );
  }, [dispatch, selectedCandidate]);

  const completeBubbleInteraction = useCallback(() => {
    dispatch(snoozeChatBubbles(undefined));
  }, [dispatch]);

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
              completeBubbleInteraction();
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
          completeBubbleInteraction();
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
        completeBubbleInteraction();
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
        completeBubbleInteraction();
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
        completeBubbleInteraction();
        dismissNotification(notification.id);
        dispatch(push({ name: "buddy" }));
        return;
      }

      if (ctrl.action.startsWith("care_")) {
        completeBubbleInteraction();
        await executeBuddyAction(ctrl, dispatch);
        dismissNotification(notification.id);
        return;
      }

      if (ctrl.action === "accept_quest") {
        completeBubbleInteraction();
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
          completeBubbleInteraction();
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
      completeBubbleInteraction,
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
