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
import { useBuddyOpportunities } from "./hooks/useBuddyOpportunities";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
import type {
  BuddyControl,
  BuddyOpportunity,
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

import styles from "./BuddyChatCompanion.module.css";

interface Props {
  chatId: string;
}

interface NotificationItem {
  id: string;
  text: string;
  source: "thread" | "runtime" | "diagnostic" | "suggestion" | "opportunity";
  controls: BuddyControl[];
  timestamp: number;
  diagnostic?: DiagnosticContext | null;
  opportunity?: BuddyOpportunity;
}

export const BuddyChatCompanion: React.FC<Props> = ({ chatId }) => {
  const dispatch = useAppDispatch();
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const threadError = useAppSelector((state) =>
    selectChatErrorById(state, chatId),
  );

  const buddy = useBuddyState();
  const { unread } = useBuddyOpportunities();
  const [opportunityIndex, setOpportunityIndex] = useState(0);
  const [chatCooldownActive, setChatCooldownActive] = useState(true);
  const executeOpportunityAction = useExecuteBuddyAction();
  const [dismissMutation] = useDismissBuddySuggestionMutation();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();

  const [dismissedIds, setDismissedIds] = useState<Set<string>>(new Set());
  const [pending, setPending] = useState(false);
  const prevChatIdRef = useRef(chatId);

  useEffect(() => {
    if (prevChatIdRef.current !== chatId) {
      prevChatIdRef.current = chatId;
      setDismissedIds(new Set());
      setOpportunityIndex(0);
    }
  }, [chatId]);

  useEffect(() => {
    setChatCooldownActive(true);
    const timer = window.setTimeout(() => {
      setChatCooldownActive(false);
    }, 60_000);
    return () => window.clearTimeout(timer);
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

  const baseNotification: NotificationItem | null = useMemo(() => {
    const chatDiagnostic =
      diagnostics.find((d) => d.chat_id === chatId) ?? null;
    const normalizedThreadError = threadError?.trim() ?? null;
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
      nowPlaying.status === "failed" &&
      !nowPlaying.dismissed
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

    if (chatDiagnostic?.error_message.trim()) {
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
        controls: activeSuggestion.controls.length
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
    errorControls,
    suggestionControls,
  ]);

  const activeOpportunities = useMemo(
    () => unread.filter((opp) => !dismissedIds.has(`opportunity-${opp.id}`)),
    [dismissedIds, unread],
  );

  useEffect(() => {
    if (activeOpportunities.length <= 1) return;
    const timer = window.setInterval(() => {
      setOpportunityIndex((index) => (index + 1) % activeOpportunities.length);
    }, 12_000);
    return () => window.clearInterval(timer);
  }, [activeOpportunities.length]);

  useEffect(() => {
    if (opportunityIndex < activeOpportunities.length) return;
    setOpportunityIndex(0);
  }, [activeOpportunities.length, opportunityIndex]);

  const topOpportunity =
    baseNotification === null && activeOpportunities.length > 0
      ? activeOpportunities[opportunityIndex % activeOpportunities.length]
      : null;

  const notification: NotificationItem | null = useMemo(() => {
    if (!topOpportunity) return baseNotification;

    return {
      id: `opportunity-${topOpportunity.id}`,
      text: opportunitySpeechText(topOpportunity),
      source: "opportunity",
      controls: opportunityActionControls(topOpportunity),
      timestamp: new Date(topOpportunity.created_at).getTime(),
      diagnostic: null,
      opportunity: topOpportunity,
    };
  }, [baseNotification, topOpportunity]);

  const isDismissed = notification ? dismissedIds.has(notification.id) : false;

  useEffect(() => {
    if (chatCooldownActive || !notification || isDismissed) return;
    const t = setTimeout(() => {
      setDismissedIds((prev) => new Set(prev).add(notification.id));
    }, 15000);
    return () => clearTimeout(t);
  }, [chatCooldownActive, notification, isDismissed]);

  const handleControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (!notification) return;

      if (notification.source === "opportunity") {
        if (!notification.opportunity) return;
        const actionIndex = getOpportunityActionIndexFromControl(ctrl);
        if (actionIndex == null) return;
        const action = getOpportunityActionFromControl(
          ctrl,
          notification.opportunity,
        );
        if (!action) return;

        if (action.kind === "dismiss") {
          setDismissedIds((prev) => {
            const next = new Set(prev);
            for (const opp of activeOpportunities) {
              next.add(`opportunity-${opp.id}`);
            }
            return next;
          });
          await Promise.allSettled(
            activeOpportunities.map((opp) => {
              const dismissAction = getOpportunityDismissAction(opp);
              return executeOpportunityAction(
                dismissAction.action,
                opp,
                dismissAction.actionIndex,
              );
            }),
          );
          setOpportunityIndex(0);
          return;
        }

        await executeOpportunityAction(
          action,
          notification.opportunity,
          actionIndex,
        );
        setDismissedIds((prev) => new Set(prev).add(notification.id));
        setOpportunityIndex((index) => index + 1);
        return;
      }

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
      executeOpportunityAction,
      activeOpportunities,
      dismissMutation,
      dismissRuntimeMutation,
      dispatch,
      chatId,
    ],
  );

  if (!enabled || chatCooldownActive) return null;
  if (!notification || isDismissed) return null;

  return (
    <div className={styles.companion}>
      <BuddyCanvas
        state={buddy.state}
        onEvent={buddy.handleCanvasEvent}
        displaySize={160}
        speechOverride={notification.text}
        speechControls={notification.controls}
        onSpeechControlClick={(ctrl) => void handleControl(ctrl)}
        bubblePosition="left"
      />
    </div>
  );
};
