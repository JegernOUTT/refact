import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Button, Text } from "@radix-ui/themes";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectBuddySnapshot,
  selectNowPlaying,
  selectBuddyDiagnostics,
  selectIsBuddyInteractiveEnabled,
  selectRuntimeQueue,
  selectBuddySuggestions,
  selectActiveSpeech,
  selectSeenNotificationIds,
  selectChatBubbleSnoozedUntil,
  selectChatBubbleImpressions,
  selectConductorGhostMessages,
  selectGoalForChat,
  dismissBuddySuggestion,
  dismissRuntimeEvent,
  clearActiveSpeech,
  markBuddyNotificationSeen,
  recordChatBubbleImpression,
  snoozeChatBubbles,
  clearExpiredChatBubbleSnooze,
  type BuddyChatBubbleClass,
} from "./buddySlice";
import { openBuddyChat, startBuddyInvestigation } from "../Chat/Thread";
import { selectApiKey, selectConfig } from "../Config/configSlice";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { push } from "../Pages/pagesSlice";
import {
  useDismissBuddySuggestionMutation,
  useDismissBuddyRuntimeEventMutation,
  useUpdateBuddySettingsMutation,
  useAnswerConductorGhostMutation,
  usePauseConductorGoalMutation,
  useResumeConductorGoalMutation,
  useSetConductorGoalAutonomyMutation,
  useManualWakeConductorGoalMutation,
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
  BuddyGhostMessage,
  ConductorGoal,
  GoalAutonomy,
} from "./types";
import { isBuddyOverlaySuppressedIssue } from "./investigation";
import { executeBuddyAction } from "./executeBuddyAction";
import { conductorGoalCounts } from "./conductor";
import {
  getOpportunityActionFromControl,
  getOpportunityActionIndexFromControl,
  getOpportunityDismissAction,
  opportunityActionControls,
  opportunitySpeechText,
} from "./buddyOpportunityActions";
import {
  isFreshErrorWithinGrace,
  isBuddyRuntimeEventVisible,
  isErrorRuntimeEvent,
} from "./buddyRuntimeEvents";
import { SIGNALS } from "./constants";
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
    | "opportunity"
    | "ghost";
  controls: BuddyControl[];
  diagnostic?: DiagnosticContext | null;
  opportunity?: BuddyOpportunity;
  ghost?: BuddyGhostMessage;
  speechIntent?: string;
  ttlMs?: number | null;
  ttlSeconds?: number;
}

interface NotificationCandidate {
  notification: NotificationItem;
  kind: BuddyChatBubbleClass;
  rank: number;
  preventsAmbientOverride?: boolean;
}

interface PinnedReactionCandidate {
  chatId: string;
  notificationId: string;
  candidate: NotificationCandidate;
  pinnedUntilMs: number;
  expiresAtMs: number;
}

const EVENT_ONCE_FRESHNESS_MS = 75_000;
const CHAT_REACTION_MIN_DISPLAY_MS = 10_000;
const AMBIENT_RATIO_TARGET = 0.5;
const AMBIENT_SIGNALS = new Set<string>([
  "speech_humor",
  "speech_insight",
  "speech_chat_reaction",
  "speech_memory_pulse_commentary",
  "speaker_insight",
  "speaker_memory_pulse_commentary",
]);
const AMBIENT_INTENTS = new Set<string>([
  "humor",
  "insight",
  "interaction_comment",
  "memory_pulse_commentary",
]);
const LIVE_CHAT_REACTION_SIGNALS = new Set<string>([
  "speech_humor",
  "speech_insight",
  "chat_bug_candidate",
  "speech_chat_reaction",
  "chat_interaction",
  "chat_interaction_comment",
  "interaction_comment",
  "live_interaction_reaction",
]);
const DURABLE_SPEECH_INTENTS = new Set<string>([
  "tour",
  "quest_accept",
  "quest_complete",
  "milestone",
  "win",
  "suggestion",
  "error_alert",
]);

function normalizedPolicyToken(value: string | null | undefined): string {
  const token =
    value
      ?.trim()
      .toLowerCase()
      .replace(/[:\s-]+/g, "_") ?? "";
  return token.startsWith("speech_") ? token.slice("speech_".length) : token;
}

function isAmbientToken(value: string | null | undefined): boolean {
  const token = normalizedPolicyToken(value);
  if (!token) return false;
  return AMBIENT_INTENTS.has(token) || AMBIENT_SIGNALS.has(token);
}

function isLiveChatReactionSignal(value: string | null | undefined): boolean {
  const token = normalizedPolicyToken(value);
  if (!token) return false;
  return LIVE_CHAT_REACTION_SIGNALS.has(token);
}

function isLiveChatReactionEvent(event: BuddyRuntimeEvent): boolean {
  return (
    event.source === "chat_reactions" ||
    isLiveChatReactionSignal(event.signal_type) ||
    isLiveChatReactionSignal(event.source) ||
    isLiveChatReactionSignal(event.dedupe_key ?? undefined)
  );
}

function isDurableSpeechToken(value: string | null | undefined): boolean {
  const token = normalizedPolicyToken(value);
  return token ? DURABLE_SPEECH_INTENTS.has(token) : false;
}

function notificationTriggerSource(
  source: NotificationItem["source"],
): "thread" | "runtime" | "diagnostic" | "suggestion" | "frontend" {
  if (source === "speech") return "runtime";
  if (source === "opportunity") return "suggestion";
  if (source === "ghost") return "runtime";
  return source;
}

function notificationIdentity(
  source: NotificationItem["source"] | "thread-error",
  id: string,
): string {
  return `${source}:${id}`;
}

function createdAtMs(value: string): number {
  return validCreatedAtMs(value) ?? 0;
}

function validCreatedAtMs(value: string): number | null {
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

function eventFreshnessMs(ttlMs: number | null | undefined): number {
  if (ttlMs != null && Number.isFinite(ttlMs) && ttlMs > 0) {
    return Math.min(EVENT_ONCE_FRESHNESS_MS, ttlMs);
  }
  return EVENT_ONCE_FRESHNESS_MS;
}

function isFreshEventOnce(createdAt: string, ttlMs?: number | null): boolean {
  const createdAtTime = validCreatedAtMs(createdAt);
  if (createdAtTime == null) return false;
  const now = Date.now();
  if (createdAtTime > now + 30_000) return false;
  return now - createdAtTime <= eventFreshnessMs(ttlMs);
}

function isDurableSpeech(activeSpeech: {
  persistent: boolean;
  ttl_seconds: number;
  speech_intent?: string;
  dedupe_key?: string;
}): boolean {
  if (activeSpeech.persistent) return true;
  return (
    isDurableSpeechToken(activeSpeech.speech_intent) ||
    isDurableSpeechToken(activeSpeech.dedupe_key)
  );
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
  if (event.bubble_policy === "ambient") return "ambient";
  if (event.bubble_policy === "durable") return "actionable";
  if (event.bubble_policy === "event_once") return "event_once";

  if (
    isAmbientToken(event.signal_type) ||
    isAmbientToken(event.source) ||
    isAmbientToken(event.dedupe_key ?? undefined) ||
    (isLiveChatReactionEvent(event) && !isErrorRuntimeEvent(event))
  ) {
    return "ambient";
  }
  if (
    isDurableSpeechToken(event.signal_type) ||
    isDurableSpeechToken(event.source) ||
    isDurableSpeechToken(event.dedupe_key ?? undefined)
  ) {
    return "actionable";
  }
  if (isErrorRuntimeEvent(event)) {
    return event.priority === "critical" || event.persistent === true
      ? "actionable"
      : "event_once";
  }
  if ((event.controls?.length ?? 0) > 0 || event.persistent === true) {
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
  const top = sorted[0];
  const ambient = sorted.find((candidate) => candidate.kind === "ambient");
  const urgent = sorted.find(
    (candidate) => candidate.preventsAmbientOverride === true,
  );
  if (top.rank < 20 && ambient?.rank !== top.rank) return top;
  if (ambient && ambientRatio(impressions) < AMBIENT_RATIO_TARGET) {
    if (urgent) return urgent;
    return ambient;
  }
  if (ambient && top.kind !== "ambient" && top.rank > 20 && !urgent) {
    return ambient;
  }
  return top;
}

function speechMatchesChat(
  activeSpeech: { chat_id?: string } | null,
  chatId: string,
): boolean {
  return !activeSpeech?.chat_id || activeSpeech.chat_id === chatId;
}

function hasChatErrorControl(controls?: BuddyControl[]): boolean {
  return (
    controls?.some(
      (control) =>
        control.action === "investigate_error" ||
        control.action === "dismiss_runtime_event",
    ) ?? false
  );
}

function isErrorAlertSpeech(
  activeSpeech: {
    speech_intent?: string;
    dedupe_key?: string;
    controls?: BuddyControl[];
  } | null,
): boolean {
  return (
    normalizedPolicyToken(activeSpeech?.speech_intent) === "error_alert" ||
    normalizedPolicyToken(activeSpeech?.dedupe_key) === "error_alert" ||
    hasChatErrorControl(activeSpeech?.controls)
  );
}

function isChatCompanionSuggestion(suggestion: BuddySuggestion): boolean {
  return suggestion.suggestion_type !== "error_pattern";
}

function isChatCompanionOpportunity(opportunity: BuddyOpportunity): boolean {
  // Diagnostic investigations are surfaced through the BuddyPanel/BuddyHome
  // hero flow rather than the chat-companion bubble. We still include any
  // opportunity that carries user-facing actions so the chat companion can
  // render accept/dismiss/investigate affordances for diagnostic kinds too.
  if (opportunity.kind === "diagnostic_investigation") {
    return opportunity.proposed_actions.length > 0;
  }
  return true;
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
        // Keep bare error events out of the bubble (no controls = no action
        // affordance) but allow error events that ship explicit user actions
        // (Investigate / Dismiss) through. `isBuddyRuntimeEventVisible`
        // already gates on freshness, persistence, and the tool-failed noise
        // rule, so we don't need to duplicate that here.
        (!isErrorRuntimeEvent(event) || (event.controls?.length ?? 0) > 0) &&
        !isBuddyOverlaySuppressedIssue(
          formatBuddyRuntimeEventText(event),
          chatDiagnostic,
        ),
    )
    .sort(compareBuddyRuntimeEvents);
}

function runtimeEventControls(
  event: BuddyRuntimeEvent,
  errorControls: BuddyControl[],
): BuddyControl[] {
  if (event.controls?.length) return event.controls;
  return isErrorRuntimeEvent(event) ? errorControls : [];
}

function isPersistentActiveProgressEvent(event: BuddyRuntimeEvent): boolean {
  if (isErrorRuntimeEvent(event)) return false;
  if (event.persistent !== true) return false;
  return (
    event.status === "started" ||
    event.status === "progress" ||
    event.status === "streaming"
  );
}

function reactionSignalForNotification(
  notification: NotificationItem | null,
  runtimeQueue: BuddyRuntimeEvent[],
  nowPlaying: BuddyRuntimeEvent | null,
): string | null {
  const event = runtimeEventForNotification(
    notification,
    runtimeQueue,
    nowPlaying,
  );
  if (!event || !isLiveChatReactionEvent(event)) return null;
  return Object.prototype.hasOwnProperty.call(SIGNALS, event.signal_type)
    ? event.signal_type
    : "speech_chat_reaction";
}

function runtimeEventForNotification(
  notification: NotificationItem | null,
  runtimeQueue: BuddyRuntimeEvent[],
  nowPlaying: BuddyRuntimeEvent | null,
): BuddyRuntimeEvent | null {
  if (notification?.source !== "runtime") return null;
  return (
    [nowPlaying, ...runtimeQueue].find(
      (candidate): candidate is BuddyRuntimeEvent =>
        candidate?.id === notification.sourceId,
    ) ?? null
  );
}

function isLiveChatReactionNotification(
  notification: NotificationItem | null,
  runtimeQueue: BuddyRuntimeEvent[],
  nowPlaying: BuddyRuntimeEvent | null,
): boolean {
  const event = runtimeEventForNotification(
    notification,
    runtimeQueue,
    nowPlaying,
  );
  return event ? isLiveChatReactionEvent(event) : false;
}

function isFreshRuntimeEventForBubble(event: BuddyRuntimeEvent): boolean {
  const createdAtTime = validCreatedAtMs(event.created_at);
  if (createdAtTime == null) return false;
  const now = Date.now();
  if (createdAtTime > now + 30_000) return false;
  return now - createdAtTime <= eventFreshnessMs(event.ttl_ms);
}

function runtimeEventFreshUntilMs(event: BuddyRuntimeEvent): number {
  const createdAtTime = validCreatedAtMs(event.created_at);
  if (createdAtTime == null) return Date.now();
  return createdAtTime + eventFreshnessMs(event.ttl_ms);
}

function isPinnedReactionVisible(
  pinned: PinnedReactionCandidate | null,
  chatId: string,
  dismissedNotificationIds: Set<string>,
  runtimeQueue: BuddyRuntimeEvent[],
  nowPlaying: BuddyRuntimeEvent | null,
): pinned is PinnedReactionCandidate {
  if (!pinned) return false;
  if (pinned.chatId !== chatId) return false;
  if (dismissedNotificationIds.has(pinned.notificationId)) {
    return false;
  }
  const event = runtimeEventForNotification(
    pinned.candidate.notification,
    runtimeQueue,
    nowPlaying,
  );
  if (event && !isBuddyRuntimeEventVisible(event)) return false;
  if (event && !isFreshRuntimeEventForBubble(event)) return false;
  const now = Date.now();
  return now < pinned.pinnedUntilMs && now <= pinned.expiresAtMs;
}

function isFreshCriticalRuntimeCandidate(
  candidate: NotificationCandidate | null,
  runtimeQueue: BuddyRuntimeEvent[],
  nowPlaying: BuddyRuntimeEvent | null,
): boolean {
  const event = runtimeEventForNotification(
    candidate?.notification ?? null,
    runtimeQueue,
    nowPlaying,
  );
  return event?.priority === "critical" && isFreshErrorWithinGrace(event);
}

function runtimeEventRank(event: BuddyRuntimeEvent, index: number): number {
  if (event.priority === "critical" && isFreshErrorWithinGrace(event)) {
    return 10 + index;
  }
  if (
    isPersistentActiveProgressEvent(event) &&
    isFreshRuntimeEventForBubble(event)
  ) {
    return 20 + index;
  }
  if (event.priority === "high" && isFreshErrorWithinGrace(event)) {
    return 25 + index;
  }
  if (isLiveChatReactionEvent(event)) return 30 + index;
  if (isErrorRuntimeEvent(event)) return 40 + index;
  if (event.priority === "critical") return 50 + index;
  if (event.priority === "high") return 55 + index;
  return 60 + index;
}

function ghostLabel(ghost: BuddyGhostMessage): string {
  if (ghost.role === "ask") return `👻 Buddy asks: ${ghost.content}`;
  if (ghost.role === "memo") return `👻 Buddy memo: ${ghost.content}`;
  return `👻 Buddy says: ${ghost.content}`;
}

function ghostControls(ghost: BuddyGhostMessage): BuddyControl[] {
  if (ghost.role !== "ask" || !ghost.question_id || !ghost.goal_id) return [];
  return [
    {
      id: `answer-${ghost.id}`,
      label: "Answer Buddy",
      action: "answer_conductor_ghost",
      style: "primary",
    },
    {
      id: `dismiss-${ghost.id}`,
      label: "Later gremlin",
      action: "dismiss",
      style: "ghost",
    },
  ];
}

const AUTONOMY_OPTIONS: { value: GoalAutonomy; label: string }[] = [
  { value: "read_only", label: "Read-only" },
  { value: "governed", label: "Governed" },
  { value: "full_auto", label: "Full auto" },
];

function formatStatusToken(value: string): string {
  return value.replace(/_/g, " ");
}

function formatShortNumber(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

function formatDuration(seconds: number): string {
  if (seconds >= 3600) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  if (seconds >= 60) return `${Math.floor(seconds / 60)}m`;
  return `${seconds}s`;
}

function budgetRatio(spent: number, limit?: number | null): number | null {
  if (!limit || limit <= 0) return null;
  return Math.max(0, Math.min(1, spent / limit));
}

function staminaPercent(goal: ConductorGoal): number {
  const ratios = [
    budgetRatio(goal.spent.elapsed_secs, goal.budget.wall_clock_secs),
    budgetRatio(goal.spent.total_tokens, goal.budget.total_tokens),
    budgetRatio(goal.spent.usd ?? 0, goal.budget.usd),
  ].filter((ratio): ratio is number => ratio != null);
  if (ratios.length === 0) return 100;
  return Math.max(0, Math.round((1 - Math.max(...ratios)) * 100));
}

function tokensLabel(goal: ConductorGoal): string {
  const spent = formatShortNumber(goal.spent.total_tokens);
  return goal.budget.total_tokens
    ? `${spent} / ${formatShortNumber(goal.budget.total_tokens)}`
    : spent;
}

function usdLabel(goal: ConductorGoal): string {
  if (goal.spent.usd == null && goal.budget.usd == null) return "—";
  const spent =
    goal.spent.usd == null ? "$0.00" : `$${goal.spent.usd.toFixed(2)}`;
  return goal.budget.usd == null
    ? spent
    : `${spent} / $${goal.budget.usd.toFixed(2)}`;
}

function waitingQuestions(goal: ConductorGoal): number {
  return goal.ledger.pending_questions.filter(
    (question) => !question.answered_at,
  ).length;
}

export const BuddyChatCompanion: React.FC<Props> = ({ chatId }) => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const enabled = useAppSelector(selectIsBuddyInteractiveEnabled);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const seenNotificationIds = useAppSelector(selectSeenNotificationIds);
  const chatBubbleSnoozedUntil = useAppSelector(selectChatBubbleSnoozedUntil);
  const chatBubbleImpressions = useAppSelector(selectChatBubbleImpressions);
  const ghostMessages = useAppSelector(selectConductorGhostMessages);
  const conductorGoal = useAppSelector((state) =>
    selectGoalForChat(state, { chatId }),
  );
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);

  const buddy = useBuddyState();
  const triggerBuddySignal = buddy.signal;
  const { unread } = useBuddyOpportunities();
  const executeOpportunityAction = useExecuteBuddyAction();
  const [dismissMutation] = useDismissBuddySuggestionMutation();
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();
  const [updateSettings, { isLoading: isEnabling }] =
    useUpdateBuddySettingsMutation();
  const [answerGhost] = useAnswerConductorGhostMutation();
  const [pauseGoal] = usePauseConductorGoalMutation();
  const [resumeGoal] = useResumeConductorGoalMutation();
  const [setGoalAutonomy] = useSetConductorGoalAutonomyMutation();
  const [manualWakeGoal] = useManualWakeConductorGoalMutation();

  const [dismissedNotificationIds, setDismissedNotificationIds] = useState<
    Set<string>
  >(new Set());
  const [activeNotificationId, setActiveNotificationId] = useState<
    string | null
  >(null);
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [pinnedReaction, setPinnedReaction] =
    useState<PinnedReactionCandidate | null>(null);
  const [ghostReply, setGhostReply] = useState("");
  const [, refreshSpeechExpiry] = useState(0);
  const pendingRef = useRef(false);
  const prevChatIdRef = useRef(chatId);
  const recordedNotificationIdsRef = useRef<Set<string>>(new Set());
  const signaledNotificationIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (prevChatIdRef.current !== chatId) {
      prevChatIdRef.current = chatId;
      setDismissedNotificationIds(new Set());
      setActiveNotificationId(null);
      setPinnedReaction(null);
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
        label: "Poke trail",
        action: "investigate_error",
        style: "primary",
      },
      {
        id: "dismiss",
        label: "Nope goblin",
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
        label: "Poke trail",
        action: "investigate_error",
        style: "primary",
      },
      {
        id: "ignore",
        label: "Nope goblin",
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
      setPinnedReaction((current) =>
        current?.notificationId === id ? null : current,
      );
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
      speechMatchesChat(activeSpeech, chatId) &&
      !isErrorAlertSpeech(activeSpeech)
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
      if (
        isLiveChatReactionEvent(event) &&
        !isFreshRuntimeEventForBubble(event)
      ) {
        continue;
      }
      const id = notificationIdentity("runtime", event.id);
      if (!isEligible(id)) continue;
      candidates.push({
        kind: classifyRuntimeEvent(event),
        rank: runtimeEventRank(event, index),
        preventsAmbientOverride:
          isFreshErrorWithinGrace(event) ||
          (isPersistentActiveProgressEvent(event) &&
            isFreshRuntimeEventForBubble(event)),
        notification: {
          id,
          sourceId: event.id,
          text: formatBuddyRuntimeEventText(event),
          createdAt: event.created_at,
          source: "runtime",
          controls: runtimeEventControls(event, errorControls),
          diagnostic: chatDiagnostic,
          ttlMs: event.ttl_ms,
        },
      });
    }

    suggestions.forEach((suggestion: BuddySuggestion, index) => {
      const id = notificationIdentity("suggestion", suggestion.id);
      if (
        suggestion.dismissed ||
        !isChatCompanionSuggestion(suggestion) ||
        !isEligible(id)
      ) {
        return;
      }
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
      .filter(
        (opportunity) =>
          isChatCompanionOpportunity(opportunity) &&
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

    ghostMessages
      .filter((ghost) => isEligible(notificationIdentity("ghost", ghost.id)))
      .forEach((ghost, index) => {
        candidates.push({
          kind: ghost.role === "ask" ? "actionable" : "event_once",
          rank: 5 + index,
          notification: {
            id: notificationIdentity("ghost", ghost.id),
            sourceId: ghost.id,
            text: ghostLabel(ghost),
            createdAt: ghost.created_at,
            source: "ghost",
            controls: ghostControls(ghost),
            diagnostic: null,
            ghost,
            ttlMs: ghost.role === "memo" ? 30_000 : null,
            speechIntent: ghost.role === "ask" ? "question" : "insight",
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
    ghostMessages,
    nowPlaying,
    runtimeQueue,
    seenNotificationIds,
    suggestionControls,
    suggestions,
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
      .flatMap((candidate) => {
        const createdAtTime = validCreatedAtMs(
          candidate.notification.createdAt,
        );
        if (createdAtTime == null) return [];
        const ttlMs =
          candidate.notification.source === "speech"
            ? (candidate.notification.ttlSeconds ?? 0) * 1000
            : candidate.notification.ttlMs;
        return [createdAtTime + eventFreshnessMs(ttlMs) - Date.now() + 1];
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

  const pickedCandidate = useMemo<NotificationCandidate | null>(() => {
    if (chatBubbleSnoozedUntil != null && chatBubbleSnoozedUntil > Date.now()) {
      return null;
    }
    const candidate = pickNotificationCandidate(
      notificationCandidates,
      chatBubbleImpressions,
    );
    const activeCandidate = notificationCandidates.find(
      (item) =>
        item.notification.id === activeNotificationId && isCandidateFresh(item),
    );
    if (activeCandidate && candidate === activeCandidate) {
      return activeCandidate;
    }
    return candidate;
  }, [
    activeNotificationId,
    chatBubbleImpressions,
    chatBubbleSnoozedUntil,
    notificationCandidates,
  ]);

  const selectedCandidate = useMemo<NotificationCandidate | null>(() => {
    if (chatBubbleSnoozedUntil != null && chatBubbleSnoozedUntil > Date.now()) {
      return null;
    }
    if (
      isPinnedReactionVisible(
        pinnedReaction,
        chatId,
        dismissedNotificationIds,
        runtimeQueue,
        nowPlaying,
      )
    ) {
      if (
        isFreshCriticalRuntimeCandidate(
          pickedCandidate,
          runtimeQueue,
          nowPlaying,
        )
      ) {
        return pickedCandidate;
      }
      return pinnedReaction.candidate;
    }
    return pickedCandidate;
  }, [
    chatBubbleSnoozedUntil,
    chatId,
    dismissedNotificationIds,
    nowPlaying,
    pickedCandidate,
    pinnedReaction,
    runtimeQueue,
  ]);

  const notification = selectedCandidate?.notification ?? null;
  const reactionSignal = useMemo(
    () => reactionSignalForNotification(notification, runtimeQueue, nowPlaying),
    [notification, nowPlaying, runtimeQueue],
  );

  useEffect(() => {
    setActionError(null);
  }, [notification?.id]);

  useEffect(() => {
    setGhostReply("");
  }, [notification?.id]);

  useEffect(() => {
    if (!notification) {
      setActiveNotificationId(null);
      return;
    }
    if (activeNotificationId === notification.id) return;
    setActiveNotificationId(notification.id);
  }, [activeNotificationId, notification]);

  useLayoutEffect(() => {
    if (!selectedCandidate) return;
    if (
      !isLiveChatReactionNotification(
        selectedCandidate.notification,
        runtimeQueue,
        nowPlaying,
      )
    ) {
      return;
    }
    const event = runtimeEventForNotification(
      selectedCandidate.notification,
      runtimeQueue,
      nowPlaying,
    );
    if (!event || !isFreshRuntimeEventForBubble(event)) return;
    setPinnedReaction((current) => {
      if (current?.notificationId === selectedCandidate.notification.id) {
        return current;
      }
      const now = Date.now();
      return {
        chatId,
        notificationId: selectedCandidate.notification.id,
        candidate: selectedCandidate,
        pinnedUntilMs: now + CHAT_REACTION_MIN_DISPLAY_MS,
        expiresAtMs: runtimeEventFreshUntilMs(event),
      };
    });
  }, [chatId, nowPlaying, runtimeQueue, selectedCandidate]);

  useEffect(() => {
    if (!pinnedReaction) return;
    const nextCheckMs = Math.min(
      pinnedReaction.pinnedUntilMs,
      pinnedReaction.expiresAtMs,
    );
    const delayMs = nextCheckMs - Date.now() + 1;
    if (delayMs <= 0) {
      refreshSpeechExpiry((tick) => tick + 1);
      return;
    }
    const timer = window.setTimeout(() => {
      refreshSpeechExpiry((tick) => tick + 1);
    }, delayMs);
    return () => window.clearTimeout(timer);
  }, [pinnedReaction]);

  useEffect(() => {
    if (!activeNotificationId) return;
    if (activeNotificationId in seenNotificationIds) return;
    dispatch(markBuddyNotificationSeen(activeNotificationId));
  }, [activeNotificationId, dispatch, seenNotificationIds]);

  useEffect(() => {
    if (!selectedCandidate) return;
    if (
      recordedNotificationIdsRef.current.has(selectedCandidate.notification.id)
    ) {
      return;
    }
    recordedNotificationIdsRef.current.add(selectedCandidate.notification.id);
    dispatch(
      recordChatBubbleImpression({
        id: selectedCandidate.notification.id,
        kind: selectedCandidate.kind,
      }),
    );
  }, [dispatch, selectedCandidate]);

  useEffect(() => {
    if (!notification || !reactionSignal) return;
    if (signaledNotificationIdRef.current === notification.id) return;
    signaledNotificationIdRef.current = notification.id;
    triggerBuddySignal(reactionSignal);
  }, [notification, reactionSignal, triggerBuddySignal]);

  const completeBubbleInteraction = useCallback(() => {
    dispatch(snoozeChatBubbles(undefined));
  }, [dispatch]);

  const handleEnable = useCallback(() => {
    void updateSettings({ enabled: true });
  }, [updateSettings]);

  const submitGhostReply = useCallback(async () => {
    if (!notification?.ghost) return;
    const ghost = notification.ghost;
    const answer = ghostReply.trim();
    if (!ghost.goal_id || !ghost.question_id || !answer || pendingRef.current) {
      return;
    }
    pendingRef.current = true;
    setPending(true);
    setActionError(null);
    try {
      await answerGhost({
        goal_id: ghost.goal_id,
        question_id: ghost.question_id,
        answer,
      }).unwrap();
      dismissNotification(notification.id);
      completeBubbleInteraction();
      setGhostReply("");
    } catch (error) {
      restoreNotification(notification.id);
      setActionError(formatOpportunityActionError(error));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }, [
    answerGhost,
    completeBubbleInteraction,
    dismissNotification,
    ghostReply,
    notification,
    restoreNotification,
  ]);

  const handleGoalAction = useCallback(
    async (action: "pause" | "resume" | "wake") => {
      if (!conductorGoal || pendingRef.current) return;
      pendingRef.current = true;
      setPending(true);
      setActionError(null);
      try {
        if (action === "pause") {
          await pauseGoal(conductorGoal.id).unwrap();
        } else if (action === "resume") {
          await resumeGoal(conductorGoal.id).unwrap();
        } else {
          await manualWakeGoal(conductorGoal.id).unwrap();
        }
      } catch (error) {
        setActionError(formatOpportunityActionError(error));
      } finally {
        pendingRef.current = false;
        setPending(false);
      }
    },
    [conductorGoal, manualWakeGoal, pauseGoal, resumeGoal],
  );

  const handleAutonomyChange = useCallback(
    async (event: React.ChangeEvent<HTMLSelectElement>) => {
      if (!conductorGoal || pendingRef.current) return;
      const autonomy = event.target.value as GoalAutonomy;
      pendingRef.current = true;
      setPending(true);
      setActionError(null);
      try {
        await setGoalAutonomy({ goal_id: conductorGoal.id, autonomy }).unwrap();
      } catch (error) {
        setActionError(formatOpportunityActionError(error));
      } finally {
        pendingRef.current = false;
        setPending(false);
      }
    },
    [conductorGoal, setGoalAutonomy],
  );

  const handleSteer = useCallback(async () => {
    if (!conductorGoal || pendingRef.current) return;
    const steering = window.prompt("Steer Buddy conductor with a short note");
    const content = steering?.trim();
    if (!content) return;
    pendingRef.current = true;
    setPending(true);
    setActionError(null);
    try {
      await sendUserMessage(chatId, content, config, apiKey ?? undefined, true);
    } catch (error) {
      setActionError(formatOpportunityActionError(error));
    } finally {
      pendingRef.current = false;
      setPending(false);
    }
  }, [apiKey, chatId, conductorGoal, config]);

  const openConductorLog = useCallback(() => {
    if (!conductorGoal) return;
    const logChatId = conductorGoal.ledger.chat_ids[0] ?? chatId;
    dispatch(openBuddyChat({ chat_id: logChatId, title: conductorGoal.title }));
    dispatch(push({ name: "chat" }));
  }, [chatId, conductorGoal, dispatch]);

  const openConductorPage = useCallback(() => {
    dispatch(push({ name: "conductor" }));
  }, [dispatch]);

  const openPlannerTask = useCallback(() => {
    const taskId = conductorGoal?.ledger.planner_task_id;
    if (!taskId) return;
    dispatch(push({ name: "task workspace", taskId }));
  }, [conductorGoal, dispatch]);

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

      if (ctrl.action === "answer_conductor_ghost") {
        await submitGhostReply();
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
      submitGhostReply,
    ],
  );

  if (!snapshot) return null;
  if (!enabled) {
    return (
      <div className={styles.disabledCompanion}>
        <Text size="1" color="gray">
          Pixel is disabled
        </Text>
        <Button
          size="1"
          variant="soft"
          onClick={handleEnable}
          disabled={isEnabling}
        >
          Enable
        </Button>
      </div>
    );
  }
  if (!notification && !conductorGoal) return null;

  const stamina = conductorGoal ? staminaPercent(conductorGoal) : 100;
  const humanYieldCount = conductorGoal ? waitingQuestions(conductorGoal) : 0;
  const goalCounts = conductorGoal ? conductorGoalCounts(conductorGoal) : null;

  return (
    <div
      className={styles.companion}
      data-notification-id={notification?.id ?? `goal:${conductorGoal?.id}`}
    >
      {conductorGoal ? (
        <section
          className={styles.cockpit}
          aria-label="Buddy conductor cockpit"
        >
          <div className={styles.cockpitHeader}>
            <div>
              <Text size="1" className={styles.cockpitKicker}>
                Conductor cockpit
              </Text>
              <Text size="2" weight="bold" className={styles.cockpitTitle}>
                {conductorGoal.title}
              </Text>
            </div>
            <span className={styles.statusBadge}>
              {formatStatusToken(conductorGoal.status)}
            </span>
          </div>

          <div className={styles.phaseRow}>
            <span>Phase: {formatStatusToken(conductorGoal.status)}</span>
            <span>Autonomy: {formatStatusToken(conductorGoal.autonomy)}</span>
            {humanYieldCount > 0 ||
            conductorGoal.status === "waiting_for_human" ? (
              <span className={styles.humanYield}>
                Human yield · {humanYieldCount}
              </span>
            ) : (
              <span className={styles.autoYield}>No human block</span>
            )}
          </div>

          <div className={styles.countRow}>
            <span>Planner {goalCounts?.planners ?? 0}</span>
            <span>Cards {goalCounts?.cards ?? 0}</span>
            <span>Agents {goalCounts?.agents ?? 0}</span>
          </div>

          <div className={styles.staminaPanel}>
            <div className={styles.staminaHeader}>
              <span>Buddy stamina</span>
              <strong>{stamina}%</strong>
            </div>
            <meter
              className={classNames(styles.staminaMeter, {
                [styles.staminaLow]: stamina < 30,
              })}
              aria-label="Buddy stamina gauge"
              min={0}
              max={100}
              value={stamina}
            />
            <div className={styles.budgetGrid}>
              <span>
                Elapsed {formatDuration(conductorGoal.spent.elapsed_secs)}
              </span>
              <span>Tokens {tokensLabel(conductorGoal)}</span>
              <span>
                Cache {formatShortNumber(conductorGoal.spent.cache_read_tokens)}
              </span>
              <span>USD {usdLabel(conductorGoal)}</span>
            </div>
          </div>

          <div className={styles.controlRow}>
            <button
              type="button"
              className={styles.controlButton}
              onClick={() => void handleGoalAction("pause")}
              disabled={pending || conductorGoal.status === "paused"}
            >
              Pause
            </button>
            <button
              type="button"
              className={styles.controlButton}
              onClick={() => void handleGoalAction("resume")}
              disabled={pending || conductorGoal.status === "running"}
            >
              Resume
            </button>
            <button
              type="button"
              className={styles.controlButton}
              onClick={() => void handleSteer()}
              disabled={pending}
            >
              Steer
            </button>
            <button
              type="button"
              className={styles.controlButton}
              onClick={() => void handleGoalAction("wake")}
              disabled={pending}
            >
              Manual wake
            </button>
            <button
              type="button"
              className={styles.controlButton}
              onClick={openPlannerTask}
              disabled={!conductorGoal.ledger.planner_task_id}
            >
              Spawn planner
            </button>
            <button
              type="button"
              className={styles.controlButton}
              onClick={openConductorPage}
            >
              Merge trigger
            </button>
            <button
              type="button"
              className={classNames(
                styles.controlButton,
                styles.primaryControl,
              )}
              onClick={openConductorLog}
            >
              Open conductor log
            </button>
            <select
              className={styles.autonomySelect}
              value={conductorGoal.autonomy}
              onChange={(event) => void handleAutonomyChange(event)}
              aria-label="Set conductor autonomy"
              disabled={pending}
            >
              {AUTONOMY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>
        </section>
      ) : null}
      {notification?.ghost?.role === "ask" ? (
        <form
          className={styles.ghostReplyForm}
          onSubmit={(event) => {
            event.preventDefault();
            void submitGhostReply();
          }}
          aria-label="Answer Buddy ask"
        >
          <input
            className={styles.ghostReplyInput}
            value={ghostReply}
            onChange={(event) => setGhostReply(event.target.value)}
            placeholder="Answer Buddy..."
            aria-label="Buddy answer"
            disabled={pending}
          />
        </form>
      ) : null}
      <BuddyCanvas
        state={buddy.state}
        onEvent={buddy.handleCanvasEvent}
        displaySize={160}
        speechOverride={
          actionError ?? notification?.text ?? "Cockpit online, tiny captain."
        }
        speechControls={notification?.controls ?? []}
        speechIntent={notification?.speechIntent ?? "insight"}
        onSpeechControlClick={(ctrl) => void handleControl(ctrl)}
        bubblePosition="left"
        compactBubble
        chatCompanionBubble
      />
    </div>
  );
};
