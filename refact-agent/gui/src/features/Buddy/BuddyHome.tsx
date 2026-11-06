import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Button,
  Flex,
  IconButton,
  LoadingState,
  Surface,
  Tabs,
  Text,
} from "../../components/ui";
import { Settings } from "lucide-react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  pop,
  push,
  selectCurrentPage,
  type BuddyPage as BuddyRouteContext,
} from "../Pages/pagesSlice";
import { BuddySettingsPanel } from "./BuddySettingsPanel";
import { BuddyWorld } from "./BuddyWorld";
import { BuddyErrorBoundary } from "./BuddyErrorBoundary";
import type { RecentBuddyError } from "./BuddyRecentErrorsPanel";
import { useBuddyState } from "./hooks/useBuddyState";
import {
  selectBuddySnapshot,
  selectBuddyLoaded,
  selectIsBuddyInteractiveEnabled,
  selectBuddyActivities,
  selectNowPlaying,
  selectActiveSpeech,
  selectBuddySuggestions,
  selectBuddyDiagnostics,
  selectRuntimeQueue,
  selectPulse,
  selectUnreadOpportunities,
  selectVerdictReaction,
  selectHomeSnoozedUntil,
  selectSeenNotificationIds,
  dismissRuntimeEvent,
  snoozeHomeNotifications,
  markBuddyNotificationSeen,
  clearExpiredBuddyNotificationSnooze,
  setActiveSpeech,
  setBuddySnapshot,
} from "./buddySlice";
import {
  openBuddyChat,
  openChatInModeAndStart,
  startBuddyInvestigation,
} from "../Chat/Thread";
import {
  buddyActionFeedbackSpeech,
  executeBuddyAction,
  navigateFromBuddyPage,
} from "./executeBuddyAction";
import {
  buildBuddySceneSpeechCandidates,
  isBuddySpeechExpired,
  pickBuddySceneSpeechCandidate,
  type BuddySceneSpeech,
} from "./buddySceneSpeech";
import { buddyActionResultMessage } from "./hooks/useExecuteBuddyAction";
import { useBuddyControlAction } from "./hooks/useBuddyControlAction";
import { computeXpFill } from "./buddyUtils";
import { buddyFailureLabel, resetBuddyPersistentState } from "./ui";
import {
  buddyOperatorIdentity,
  useGetBuddyOperatorStatusQuery,
  useGetBuddyLegacyDraftWorkQuery,
  useReopenBuddyOperatorMutation,
} from "../../services/refact/buddyOperator";
import { BuddyNativeDecisions } from "./BuddyNativeDecisions";
import { BuddyNativeApprovals } from "./BuddyNativeApprovals";
import { BuddySupervision } from "./BuddySupervision";
import {
  requiresEditorialClaim,
  useBuddyEditorialDelivery,
} from "./useBuddyEditorialDelivery";
import {
  getOpportunityActionFromControl,
  getOpportunityActionIndexFromControl,
} from "./buddyOpportunityActions";
import {
  buddyApi,
  useDismissBuddyRuntimeEventMutation,
  useUpdateBuddySettingsMutation,
} from "../../services/refact/buddy";
import type {
  BuddyCareAction,
  BuddyControl,
  BuddyPage,
  BuddyRuntimeEvent,
  DiagnosticContext,
} from "./types";
import { PALETTES, STAGES } from "./constants";
import { useGetStatsSummaryQuery } from "../../services/refact/stats";
import { useGetSetupStatusQuery } from "../../services/refact/setupStatus";
import { BuddyPresenceProvider } from "./BuddyPresenceProvider";
import type { BuddyHomeWorkMode } from "./BuddyHomeOverview";
import {
  BuddyDegradedBanner,
  BuddyPageHeader,
  BuddyStatusPill,
} from "./sections/BuddyPageHeader";
import { BuddyHomeSection as BuddyHomeOverviewSection } from "./sections/BuddyHomeSection";
import { BuddyReportsSection } from "./sections/BuddyReportsSection";
import { BuddyActivitySection } from "./sections/BuddyActivitySection";
import { BuddyPlaySection } from "./sections/BuddyPlaySection";
import { BuddySettingsSection } from "./sections/BuddySettingsSection";
import type {
  BuddyHomeSection,
  BuddyOperatorFailure,
  BuddyOperatorMode,
} from "./sections/shared";
import styles from "./BuddyHome.module.css";

const SECTION_ORDER: BuddyHomeSection[] = [
  "home",
  "reports",
  "activity",
  "play",
  "settings",
];

const SECTION_LABELS: Record<BuddyHomeSection, string> = {
  home: "Home",
  reports: "Reports",
  activity: "Activity",
  play: "Play",
  settings: "Settings",
};

const RECENT_ERROR_WINDOW_MS = 6 * 60 * 60 * 1000;
const RECENT_ERROR_REFRESH_MS = 60 * 1000;

function normalizeRecentErrorText(value: string | null | undefined): string {
  return (value ?? "").trim().replace(/\s+/g, " ");
}

function recentErrorDetail(event: RecentBuddyError): string | null {
  const normalizedTitle = normalizeRecentErrorText(event.title).toLowerCase();
  return (
    [event.failure_summary, event.description]
      .map(normalizeRecentErrorText)
      .find((detail) => {
        const normalizedDetail = detail.toLowerCase();
        return (
          detail &&
          normalizedDetail !== normalizedTitle &&
          !normalizedDetail.startsWith(normalizedTitle)
        );
      }) ?? null
  );
}

function uniqueRecentErrorIds(event: RecentBuddyError): string[] {
  const ids =
    event.relatedIds && event.relatedIds.length > 0
      ? event.relatedIds
      : [event.id];
  return Array.from(new Set(ids));
}

function isGroupedRecentError(event: RecentBuddyError): boolean {
  return (event.relatedIds?.length ?? 0) > 1 || (event.occurrences ?? 1) > 1;
}

export interface BuddyHomeProps {
  embedded?: boolean;
  routeContext?: BuddyRouteContext;
  onBack?: () => void;
}

const BuddyHomeContent: React.FC<BuddyHomeProps> = ({
  embedded = false,
  routeContext,
  onBack,
}) => {
  const dispatch = useAppDispatch();
  const globalPage = useAppSelector(selectCurrentPage);
  const currentPage = routeContext ?? (embedded ? undefined : globalPage);
  const draftId =
    currentPage?.name === "buddy" ? currentPage.draftId : undefined;
  const snapshot = useAppSelector(selectBuddySnapshot);
  const operatorIdentity = useAppSelector(buddyOperatorIdentity);
  const previousIdentity = useRef(operatorIdentity);
  useEffect(() => {
    if (previousIdentity.current !== operatorIdentity) {
      previousIdentity.current = operatorIdentity;
      resetBuddyPersistentState();
    }
  }, [operatorIdentity]);
  const operator = useGetBuddyOperatorStatusQuery(operatorIdentity, {
    refetchOnMountOrArgChange: true,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const [reopenOperator, { isLoading: isReopeningOperator }] =
    useReopenBuddyOperatorMutation();
  const operatorMode: BuddyOperatorMode = operator.error
    ? "error"
    : operator.currentData
      ? operator.currentData.mode
      : "loading";
  const legacyCompatibility = operatorMode === "disabled";
  const operatorEnabled = operatorMode === "enabled";
  const degraded: BuddyOperatorFailure | null =
    operatorMode === "unavailable" || operatorMode === "error"
      ? operatorMode
      : null;
  const operatorNeedsRetry = degraded !== null;
  const statusPill = (
    <BuddyStatusPill operatorMode={operatorMode} degraded={degraded} />
  );
  const mappedDraft = useGetBuddyLegacyDraftWorkQuery(
    {
      id: draftId ?? "",
      project_id: operator.currentData?.project?.project_id,
      endpoint_identity: operatorIdentity,
    },
    { skip: !operatorEnabled || !draftId },
  );
  const loaded = useAppSelector(selectBuddyLoaded);
  const enabled = useAppSelector(selectIsBuddyInteractiveEnabled);
  const activities = useAppSelector(selectBuddyActivities);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const rawSpeech = useAppSelector(selectActiveSpeech);
  const editorial = useBuddyEditorialDelivery(
    rawSpeech && !isBuddySpeechExpired(rawSpeech) ? rawSpeech : null,
    "panel",
    enabled && loaded && !!snapshot,
  );
  const activeSpeech = requiresEditorialClaim(rawSpeech) ? null : rawSpeech;
  const suggestions = useAppSelector(selectBuddySuggestions);
  const recentDiagnostics = useAppSelector(selectBuddyDiagnostics);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const pulse = useAppSelector(selectPulse);
  const unreadOpportunities = useAppSelector(selectUnreadOpportunities);
  const homeSnoozedUntil = useAppSelector(selectHomeSnoozedUntil);
  const seenNotificationIds = useAppSelector(selectSeenNotificationIds);
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();
  const { control: runControl, opportunity: runOpportunity } =
    useBuddyControlAction();
  const buddy = useBuddyState();
  const { state, signal: buddySignal } = buddy;
  const [setupDismissed, setSetupDismissed] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const deepLinkKey =
    currentPage?.name === "buddy" &&
    Boolean(currentPage.workId ?? currentPage.draftId)
      ? JSON.stringify([
          currentPage.workId,
          currentPage.workProjectId,
          currentPage.draftId,
        ])
      : null;
  const [section, setSection] = useState<BuddyHomeSection>(
    deepLinkKey ? "reports" : "home",
  );
  const [seenDeepLink, setSeenDeepLink] = useState(deepLinkKey);
  const [selectedWork, setSelectedWork] = useState<{
    workId: string;
    projectId: string;
  } | null>(null);
  if (seenDeepLink !== deepLinkKey) {
    setSeenDeepLink(deepLinkKey);
    setSelectedWork(null);
    if (deepLinkKey) setSection("reports");
  }
  const contentRef = useRef<HTMLElement>(null);
  const focusSection = useRef(false);
  const [worldBoundaryKey, setWorldBoundaryKey] = useState(0);
  const verdictReaction = useAppSelector(selectVerdictReaction);
  const [speechIndex, setSpeechIndex] = useState(0);
  const [recentErrorNow, setRecentErrorNow] = useState(() => Date.now());
  const [isRetryingBuddySnapshot, setIsRetryingBuddySnapshot] = useState(false);
  const [operatorRetryOutcome, setOperatorRetryOutcome] = useState<
    "recovered" | "unavailable" | "failed" | null
  >(null);
  const [updateSettings, { isLoading: isSavingSettings }] =
    useUpdateBuddySettingsMutation();

  const selectSection = useCallback(
    (next: BuddyHomeSection, moveFocus = true) => {
      focusSection.current = moveFocus;
      setSection(next);
      if (contentRef.current) contentRef.current.scrollTop = 0;
    },
    [],
  );

  useEffect(() => {
    if (contentRef.current) contentRef.current.scrollTop = 0;
    if (!focusSection.current) return;
    focusSection.current = false;
    const timer = window.setTimeout(() => {
      contentRef.current
        ?.querySelector<HTMLElement>(`[data-section-panel="${section}"]`)
        ?.focus({ preventScroll: true });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [section]);

  const handleOpenWork = useCallback(
    (workId: string, projectId: string) => {
      setSelectedWork({ workId, projectId });
      selectSection("reports");
    },
    [selectSection],
  );

  const handleShowReports = useCallback(() => {
    setSelectedWork(null);
    selectSection("reports");
  }, [selectSection]);

  const homeWorkMode: BuddyHomeWorkMode = operatorEnabled
    ? "enabled"
    : legacyCompatibility
      ? "legacy"
      : degraded
        ? "degraded"
        : "loading";

  const { data: statsData } = useGetStatsSummaryQuery({});
  const { data: setupData } = useGetSetupStatusQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const setupNeeded = !setupData?.configured && !setupDismissed;

  const paletteIndex =
    snapshot?.state.identity.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const progression = snapshot?.state.progression;
  const identity = snapshot?.state.identity;
  const skills = snapshot?.state.skills;
  const pet = snapshot?.state.pet;
  const personality = snapshot?.state.personality;
  const settings = snapshot?.settings;
  const activeQuest = snapshot?.state.active_quest ?? null;

  const stageIndex = progression?.stage ?? state.progress.stage;
  const stage = STAGES[stageIndex] ?? STAGES[0];
  const nextStage = STAGES.at(stageIndex + 1);
  const atMaxStage = stageIndex >= STAGES.length - 1;

  const xp = progression?.xp ?? state.progress.xp;
  const xpNext =
    progression?.xp_next !== undefined && progression.xp_next > 0
      ? progression.xp_next
      : nextStage?.xpThreshold;
  const xpFill = useMemo(() => computeXpFill(xp, xpNext ?? 0), [xp, xpNext]);

  const name = identity?.name ?? state.name;

  const successRate = useMemo(() => {
    if (!statsData || statsData.totals.total_calls === 0) return null;
    return Math.round(
      (statsData.totals.successful_calls / statsData.totals.total_calls) * 100,
    );
  }, [statsData]);

  const handleBack = useCallback(() => {
    if (onBack) onBack();
    else if (!embedded) dispatch(pop());
  }, [dispatch, embedded, onBack]);

  const handleEnable = useCallback(() => {
    void updateSettings({ enabled: true });
  }, [updateSettings]);

  const handleRetryBuddySnapshot = useCallback(async () => {
    setIsRetryingBuddySnapshot(true);
    try {
      const nextSnapshot = await dispatch(
        buddyApi.endpoints.getBuddySnapshot.initiate(undefined, {
          forceRefetch: true,
        }),
      ).unwrap();
      dispatch(setBuddySnapshot(nextSnapshot));
    } catch {
      return;
    } finally {
      setIsRetryingBuddySnapshot(false);
    }
  }, [dispatch]);

  const handleRetryOperator = useCallback(async () => {
    setOperatorRetryOutcome(null);
    try {
      const status = await reopenOperator(operatorIdentity).unwrap();
      setOperatorRetryOutcome(
        status.mode === "enabled" ? "recovered" : "unavailable",
      );
    } catch (error) {
      void error;
      setOperatorRetryOutcome("failed");
    } finally {
      void operator.refetch();
    }
  }, [operator, operatorIdentity, reopenOperator]);

  const handleViewStats = useCallback(() => {
    dispatch(push({ name: "stats dashboard" }));
  }, [dispatch]);

  const handleOpenWorldPage = useCallback(
    (page: BuddyPage) => {
      navigateFromBuddyPage(page, dispatch);
    },
    [dispatch],
  );

  const handleRunMode = useCallback(
    (mode: string) => {
      void dispatch(openChatInModeAndStart({ mode }));
    },
    [dispatch],
  );

  const handleDismissSetup = useCallback(() => {
    setSetupDismissed(true);
  }, []);

  const handleResetWorld = useCallback(() => {
    setWorldBoundaryKey((key) => key + 1);
  }, []);

  const handleCare = useCallback(
    async (action: BuddyCareAction, toy?: string) => {
      buddySignal(`care_${action}`);
      await executeBuddyAction(
        {
          id: `care-${action}`,
          label: action,
          action: `care_${action}`,
          action_param: toy,
          style: "primary",
        },
        dispatch,
      );
    },
    [buddySignal, dispatch],
  );

  const handleReroll = useCallback(async () => {
    await executeBuddyAction(
      {
        id: "reroll-personality",
        label: "Reroll",
        action: "reroll_personality",
        style: "primary",
      },
      dispatch,
    );
  }, [dispatch]);

  const activeSuggestion = useMemo(
    () => suggestions.find((suggestion) => !suggestion.dismissed) ?? null,
    [suggestions],
  );

  useEffect(() => {
    dispatch(clearExpiredBuddyNotificationSnooze());
    if (homeSnoozedUntil == null) return;
    const remainingMs = homeSnoozedUntil - Date.now();
    if (remainingMs <= 0) return;
    const timer = window.setTimeout(() => {
      dispatch(clearExpiredBuddyNotificationSnooze());
    }, remainingMs);
    return () => window.clearTimeout(timer);
  }, [dispatch, homeSnoozedUntil]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setRecentErrorNow(Date.now());
    }, RECENT_ERROR_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, []);

  const homeNotificationsSnoozed =
    homeSnoozedUntil != null && homeSnoozedUntil > Date.now();
  const heroSpeechCandidates = useMemo(() => {
    if (!legacyCompatibility) return [];
    if (activeSpeech && !isBuddySpeechExpired(activeSpeech)) {
      return [
        {
          id: `speech-${activeSpeech.id}`,
          text: activeSpeech.text,
          controls: activeSpeech.controls,
          chat_id: activeSpeech.chat_id,
          speech_intent: activeSpeech.speech_intent,
          source: "speech",
        } satisfies BuddySceneSpeech,
      ];
    }
    if (homeNotificationsSnoozed) return [];
    return buildBuddySceneSpeechCandidates({
      nowPlaying: requiresEditorialClaim(nowPlaying) ? null : nowPlaying,
      runtimeQueue: runtimeQueue.filter(
        (event) => !requiresEditorialClaim(event),
      ),
      activeSuggestion,
      activeOpportunities: unreadOpportunities,
    }).filter((speech) => !(speech.id in seenNotificationIds));
  }, [
    legacyCompatibility,
    activeSpeech,
    activeSuggestion,
    homeNotificationsSnoozed,
    nowPlaying,
    runtimeQueue,
    seenNotificationIds,
    unreadOpportunities,
  ]);

  useEffect(() => {
    if (heroSpeechCandidates.length <= 1) return;
    const minMs = 18_000;
    const jitterMs = Math.floor(Math.random() * 12_000);
    const timer = window.setTimeout(() => {
      setSpeechIndex((index) => (index + 1) % heroSpeechCandidates.length);
    }, minMs + jitterMs);
    return () => window.clearTimeout(timer);
  }, [heroSpeechCandidates.length, speechIndex]);

  useEffect(() => {
    if (speechIndex < heroSpeechCandidates.length) return;
    setSpeechIndex(0);
  }, [heroSpeechCandidates.length, speechIndex]);

  const heroSpeech = useMemo(() => {
    if (heroSpeechCandidates.length === 0) return null;
    if (activeSpeech && !isBuddySpeechExpired(activeSpeech)) {
      return heroSpeechCandidates[0] ?? null;
    }
    return pickBuddySceneSpeechCandidate([
      heroSpeechCandidates[speechIndex % heroSpeechCandidates.length],
    ]);
  }, [activeSpeech, heroSpeechCandidates, speechIndex]);

  const activeDiagnostic = heroSpeech?.chat_id
    ? recentDiagnostics.find((diag) => diag.chat_id === heroSpeech.chat_id)
    : undefined;

  const handleSpeechControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (!heroSpeech) return;
      if (
        heroSpeech.source === "runtime" &&
        heroSpeech.runtimeEventId &&
        (ctrl.action === "dismiss" ||
          ctrl.action === "dismiss_speech" ||
          ctrl.action === "dismiss_runtime_event")
      ) {
        void dismissRuntimeMutation(heroSpeech.runtimeEventId)
          .unwrap()
          .catch(() => undefined);
        dispatch(markBuddyNotificationSeen(heroSpeech.id));
        dispatch(snoozeHomeNotifications(undefined));
        dispatch(dismissRuntimeEvent(heroSpeech.runtimeEventId));
        return;
      }
      if (heroSpeech.source === "opportunity" && heroSpeech.opportunityId) {
        const opportunity = unreadOpportunities.find(
          (opp) => opp.id === heroSpeech.opportunityId,
        );
        const actionIndex = getOpportunityActionIndexFromControl(ctrl);
        if (!opportunity || actionIndex == null) return;
        const action = getOpportunityActionFromControl(ctrl, opportunity);
        if (!action) return;
        const result = await runOpportunity(
          action,
          opportunity,
          actionIndex,
          undefined,
          "home-speech",
        );
        if (result.status === "ignored") return;
        if (result.status !== "success") {
          dispatch(
            setActiveSpeech(
              buddyActionFeedbackSpeech(
                "buddy-action-error",
                buddyActionResultMessage(result) ?? "Action failed.",
              ),
            ),
          );
          return;
        }
        dispatch(markBuddyNotificationSeen(heroSpeech.id));
        if (action.kind === "dismiss") {
          dispatch(snoozeHomeNotifications(undefined));
        }
        return;
      }
      const result = await runControl(
        ctrl,
        {
          triggerText: heroSpeech.text,
          triggerSource:
            heroSpeech.source === "suggestion" ? "suggestion" : "runtime",
          sourceChatId: heroSpeech.chat_id,
          diagnostic: activeDiagnostic,
          suggestionId: heroSpeech.suggestionId,
        },
        "home-speech",
      );
      if (result.status === "ignored") return;
      if (result.status !== "success") {
        if (result.status !== "unsupported") {
          dispatch(
            setActiveSpeech(
              buddyActionFeedbackSpeech(
                "buddy-action-error",
                buddyActionResultMessage(result) ?? "Action failed.",
              ),
            ),
          );
        }
        return;
      }
      dispatch(markBuddyNotificationSeen(heroSpeech.id));
      if (ctrl.action === "dismiss_suggestion" && heroSpeech.suggestionId) {
        dispatch(snoozeHomeNotifications(undefined));
      }
    },
    [
      activeDiagnostic,
      dismissRuntimeMutation,
      dispatch,
      heroSpeech,
      runControl,
      runOpportunity,
      unreadOpportunities,
    ],
  );

  const handleQuestControl = useCallback(
    async (ctrl: BuddyControl) => {
      await executeBuddyAction(ctrl, dispatch, {
        triggerText: activeQuest?.title ?? `${name} quest`,
        triggerSource: "suggestion",
      });
    },
    [activeQuest?.title, dispatch, name],
  );

  const handleOpenActivityChat = useCallback(
    (chatId: string, title: string) => {
      dispatch(openBuddyChat({ chat_id: chatId, title }));
      dispatch(push({ name: "chat" }));
    },
    [dispatch],
  );

  const unlockedSkills = skills?.unlocked ?? state.skills;

  const operatorRecentErrors = useMemo<RecentBuddyError[]>(() => {
    const cutoff = recentErrorNow - RECENT_ERROR_WINDOW_MS;
    const runtimeEvents: BuddyRuntimeEvent[] = [];
    if (
      nowPlaying &&
      (nowPlaying.status === "failed" ||
        nowPlaying.priority === "critical" ||
        nowPlaying.priority === "high")
    ) {
      runtimeEvents.push(nowPlaying);
    }
    for (const event of runtimeQueue) {
      if (
        event.status === "failed" ||
        event.priority === "critical" ||
        event.priority === "high"
      ) {
        if (!runtimeEvents.some((candidate) => candidate.id === event.id)) {
          runtimeEvents.push(event);
        }
      }
    }

    const isWithinWindow = (timestamp: string): boolean => {
      const time = Date.parse(timestamp);
      return Number.isFinite(time) && time >= cutoff;
    };
    const signature = (parts: (string | null | undefined)[]) =>
      parts
        .map((part) => normalizeRecentErrorText(part).toLowerCase())
        .join("|");
    const collected = runtimeEvents.filter(
      (event) =>
        (isWithinWindow(event.created_at) ||
          ["started", "progress", "streaming"].includes(event.status)) &&
        event.status !== "dismissed",
    );
    collected.sort(
      (left, right) =>
        Date.parse(right.created_at) - Date.parse(left.created_at),
    );

    const sigMap = new Map<string, RecentBuddyError>();
    for (const event of collected) {
      const sig = signature([
        event.source,
        event.signal_type,
        event.title,
        event.description,
        event.failure_category,
        event.failure_summary,
      ]);
      const existing = sigMap.get(sig);
      if (existing) {
        const existingCreatedAt = Date.parse(existing.created_at);
        const incomingCreatedAt = Date.parse(event.created_at);
        const incomingIsNewer = incomingCreatedAt > existingCreatedAt;
        existing.occurrences = (existing.occurrences ?? 1) + 1;
        existing.dismissedAny =
          Boolean(existing.dismissedAny) || Boolean(event.dismissed);
        existing.dismissedAll =
          Boolean(existing.dismissedAll) && Boolean(event.dismissed);
        existing.relatedIds = Array.from(
          new Set([...(existing.relatedIds ?? [existing.id]), event.id]),
        );
        existing.failure_category =
          event.failure_category ?? existing.failure_category;
        existing.failure_summary =
          event.failure_summary ?? existing.failure_summary;
        if (incomingIsNewer) {
          existing.description = event.description ?? existing.description;
          existing.chat_id = event.chat_id ?? existing.chat_id;
          existing.created_at = event.created_at;
        }
        existing.dismissed =
          Boolean(existing.dismissed) || Boolean(event.dismissed);
      } else {
        sigMap.set(sig, {
          ...event,
          occurrences: 1,
          dismissedAny: Boolean(event.dismissed),
          dismissedAll: Boolean(event.dismissed),
          relatedIds: [event.id],
        });
      }
    }

    const runtimeEventsByDiagnosticKey = new Map<string, RecentBuddyError>();
    for (const event of sigMap.values()) {
      if (event.chat_id == null) continue;
      for (const value of [
        event.title,
        event.description,
        event.failure_summary,
      ]) {
        const messageKey = normalizeRecentErrorText(value).toLowerCase();
        if (!messageKey) continue;
        runtimeEventsByDiagnosticKey.set(
          signature(["cross-source", event.chat_id, messageKey]),
          event,
        );
      }
    }

    const diagnosticGroups = new Map<
      string,
      { diagnostic: DiagnosticContext; occurrences: number }
    >();
    for (const diagnostic of recentDiagnostics) {
      if (!isWithinWindow(diagnostic.collected_at)) continue;
      const messageKey = normalizeRecentErrorText(
        diagnostic.error_message,
      ).toLowerCase();
      if (!messageKey) continue;
      const dedupeKey = signature([
        diagnostic.chat_id,
        diagnostic.error_type,
        messageKey,
      ]);
      const existing = diagnosticGroups.get(dedupeKey);
      if (existing) {
        if (
          diagnostic.diagnostic_id &&
          diagnostic.diagnostic_id === existing.diagnostic.diagnostic_id
        ) {
          continue;
        }
        existing.occurrences += diagnostic.occurrences ?? 1;
      } else {
        diagnosticGroups.set(dedupeKey, {
          diagnostic,
          occurrences: diagnostic.occurrences ?? 1,
        });
      }
    }

    for (const { diagnostic, occurrences } of diagnosticGroups.values()) {
      const messageKey = normalizeRecentErrorText(
        diagnostic.error_message,
      ).toLowerCase();
      const crossSourceKey = signature([
        "cross-source",
        diagnostic.chat_id,
        messageKey,
      ]);
      const matchingRuntime =
        diagnostic.chat_id == null
          ? undefined
          : runtimeEventsByDiagnosticKey.get(crossSourceKey);
      if (matchingRuntime) {
        matchingRuntime.occurrences = Math.max(
          matchingRuntime.occurrences ?? 1,
          occurrences,
        );
        continue;
      }

      const title = diagnostic.error_type.trim()
        ? buddyFailureLabel(diagnostic.error_type)
        : diagnostic.error_message;
      const id =
        diagnostic.diagnostic_id ??
        `diagnostic-${signature([
          diagnostic.chat_id,
          diagnostic.error_type,
          messageKey,
          diagnostic.collected_at,
        ])}`;
      const event: RecentBuddyError = {
        id,
        signal_type: diagnostic.error_type,
        title,
        description: diagnostic.error_message,
        source: "diagnostic",
        status: "failed",
        priority:
          diagnostic.severity === "critical"
            ? "critical"
            : diagnostic.severity === "high"
              ? "high"
              : "normal",
        created_at: diagnostic.collected_at,
        chat_id: diagnostic.chat_id ?? undefined,
        occurrences,
        dismissed: false,
        dismissedAny: false,
        dismissedAll: false,
        relatedIds: [id],
      };
      sigMap.set(
        signature([
          "diagnostic",
          diagnostic.chat_id,
          diagnostic.error_type,
          messageKey,
        ]),
        event,
      );
      if (diagnostic.chat_id != null) {
        runtimeEventsByDiagnosticKey.set(crossSourceKey, event);
      }
    }

    return Array.from(sigMap.values());
  }, [nowPlaying, recentDiagnostics, recentErrorNow, runtimeQueue]);
  const legacyRecentErrors = useMemo<RecentBuddyError[]>(() => {
    const collected: BuddyRuntimeEvent[] = [];
    if (
      nowPlaying &&
      (nowPlaying.status === "failed" ||
        nowPlaying.priority === "critical" ||
        nowPlaying.priority === "high")
    ) {
      collected.push(nowPlaying);
    }
    for (const event of runtimeQueue) {
      if (
        event.status === "failed" ||
        event.priority === "critical" ||
        event.priority === "high"
      ) {
        if (!collected.find((candidate) => candidate.id === event.id)) {
          collected.push(event);
        }
      }
    }
    collected.sort((left, right) => {
      const leftTime = new Date(left.created_at).getTime() || 0;
      const rightTime = new Date(right.created_at).getTime() || 0;
      return rightTime - leftTime;
    });

    const cutoff = recentErrorNow - RECENT_ERROR_WINDOW_MS;
    const sigMap = new Map<string, RecentBuddyError>();
    for (const event of collected) {
      const createdAt = Date.parse(event.created_at);
      if (
        Number.isFinite(createdAt) &&
        createdAt < cutoff &&
        event.status !== "started" &&
        event.status !== "progress" &&
        event.status !== "streaming"
      ) {
        continue;
      }
      const signature = `${event.source}|${event.signal_type}|${event.title}|${
        event.description ?? ""
      }|${event.failure_category ?? ""}|${event.failure_summary ?? ""}`;
      const existing = sigMap.get(signature);
      if (existing) {
        const existingCreatedAt = Date.parse(existing.created_at);
        const incomingIsNewer =
          Number.isFinite(createdAt) &&
          (!Number.isFinite(existingCreatedAt) ||
            createdAt > existingCreatedAt);
        existing.occurrences = (existing.occurrences ?? 1) + 1;
        existing.dismissedAny =
          Boolean(existing.dismissedAny) || Boolean(event.dismissed);
        existing.dismissedAll =
          Boolean(existing.dismissedAll) && Boolean(event.dismissed);
        existing.relatedIds = Array.from(
          new Set([...(existing.relatedIds ?? [existing.id]), event.id]),
        );
        existing.failure_category =
          event.failure_category ?? existing.failure_category;
        existing.failure_summary =
          event.failure_summary ?? existing.failure_summary;
        if (incomingIsNewer) {
          existing.description = event.description ?? existing.description;
          existing.chat_id = event.chat_id ?? existing.chat_id;
          existing.created_at = event.created_at;
        }
        existing.dismissed =
          Boolean(existing.dismissed) || Boolean(event.dismissed);
      } else {
        sigMap.set(signature, {
          ...event,
          occurrences: 1,
          dismissedAny: Boolean(event.dismissed),
          dismissedAll: Boolean(event.dismissed),
          relatedIds: [event.id],
        });
      }
    }
    return Array.from(sigMap.values());
  }, [nowPlaying, recentErrorNow, runtimeQueue]);
  const recentErrorRows = useMemo(() => {
    const rows: {
      event: RecentBuddyError;
      detail: string | null;
      count: number;
    }[] = [];
    for (const event of operatorRecentErrors) {
      const detail = recentErrorDetail(event);
      const previous = rows.at(-1);
      if (
        previous &&
        normalizeRecentErrorText(previous.event.title).toLowerCase() ===
          normalizeRecentErrorText(event.title).toLowerCase() &&
        previous.detail?.toLowerCase() === detail?.toLowerCase()
      ) {
        previous.count += event.occurrences ?? 1;
      } else {
        rows.push({ event, detail, count: event.occurrences ?? 1 });
      }
    }
    return rows;
  }, [operatorRecentErrors]);

  const handleInvestigateError = useCallback(
    (event: RecentBuddyError) => {
      const triggerText = event.description
        ? `${event.title}: ${event.description}`
        : event.title;
      const diagnostic =
        event.chat_id != null
          ? recentDiagnostics.find((d) => d.chat_id === event.chat_id) ?? null
          : null;
      void dispatch(
        startBuddyInvestigation({
          triggerText,
          triggerSource: "runtime",
          sourceChatId: event.chat_id,
          diagnostic,
        }),
      );
      const shouldDismiss = !event.dismissed || isGroupedRecentError(event);
      if (shouldDismiss) {
        for (const id of uniqueRecentErrorIds(event)) {
          dispatch(dismissRuntimeEvent(id));
          void dismissRuntimeMutation(id)
            .unwrap()
            .catch(() => undefined);
        }
      }
    },
    [dispatch, recentDiagnostics, dismissRuntimeMutation],
  );

  const handleDismissError = useCallback(
    (event: RecentBuddyError) => {
      for (const id of uniqueRecentErrorIds(event)) {
        dispatch(dismissRuntimeEvent(id));
        void dismissRuntimeMutation(id)
          .unwrap()
          .catch(() => undefined);
      }
      dispatch(snoozeHomeNotifications(undefined));
    },
    [dispatch, dismissRuntimeMutation],
  );

  if (!loaded) {
    return (
      <div className={classNames(styles.page, "rf-enter")}>
        <BuddyNativeDecisions />
        <BuddyNativeApprovals />
        <BuddySupervision />
        <LoadingState label="Loading Buddy" variant="full" />
      </div>
    );
  }

  if (snapshot === null) {
    return (
      <div className={classNames(styles.page, "rf-enter")}>
        <BuddyPageHeader onBack={handleBack} />
        <Flex
          align="center"
          justify="center"
          direction="column"
          gap="2"
          className={styles.unavailableBody}
        >
          <Text size="2" color="gray">
            {name} is not available
          </Text>
          <BuddyNativeDecisions />
          <BuddyNativeApprovals />
          <BuddySupervision />
          <Button
            type="button"
            size="sm"
            variant="primary"
            onClick={() => void handleRetryBuddySnapshot()}
            disabled={isRetryingBuddySnapshot}
          >
            {isRetryingBuddySnapshot ? "Retrying…" : "Retry"}
          </Button>
        </Flex>
      </div>
    );
  }

  if (!enabled) {
    return (
      <div className={classNames(styles.page, "rf-enter")}>
        <BuddyPageHeader
          onBack={handleBack}
          identity={{ name, statusPill }}
          trailing={
            <IconButton
              variant="ghost"
              size="sm"
              aria-label="Settings"
              icon={Settings}
              onClick={() => setShowSettings((v) => !v)}
            />
          }
        />
        <main
          className={classNames(styles.content, styles.contentPadded)}
          data-testid="buddy-home-disabled"
        >
          <BuddyNativeDecisions />
          <BuddyNativeApprovals />
          <BuddySupervision />
          <Surface
            variant="glass"
            radius="card"
            animated="rise"
            className={styles.disabledState}
          >
            <Text size="2" color="gray" align="center">
              {name} is disabled. Still here, just politely lurking.
            </Text>
            <Button
              size="md"
              variant="primary"
              onClick={handleEnable}
              disabled={isSavingSettings}
            >
              Enable {name}
            </Button>
          </Surface>
          {showSettings && (
            <div
              className={classNames(styles.settingsSection, "rf-expand-grid")}
              data-state="open"
              data-testid="buddy-home-settings-section"
            >
              <BuddySettingsPanel onClose={() => setShowSettings(false)} />
            </div>
          )}
        </main>
      </div>
    );
  }

  const projectRoot = operator.currentData?.project?.canonical_root;
  const projectName = projectRoot
    ? projectRoot.split(/[\\/]/).filter(Boolean).at(-1) ?? projectRoot
    : null;
  const pageWorkId =
    currentPage?.name === "buddy"
      ? currentPage.workId ?? mappedDraft.currentData?.data.value.id
      : undefined;
  const pageProjectId =
    currentPage?.name === "buddy"
      ? currentPage.workProjectId ?? mappedDraft.currentData?.project.project_id
      : undefined;
  const renderWorld = (play: boolean) => (
    <BuddyErrorBoundary key={worldBoundaryKey}>
      <BuddyWorld
        homeDoorDisabled
        compact={!play}
        portrait={!play}
        palette={palette}
        stage={stage}
        state={state}
        pulse={pulse}
        pet={pet}
        nowPlaying={nowPlaying}
        activeQuest={activeQuest}
        onCanvasEvent={buddy.handleCanvasEvent}
        activeSpeech={play ? heroSpeech : null}
        setupNeeded={setupNeeded}
        onRunMode={handleRunMode}
        onDismissSetup={handleDismissSetup}
        onCare={(action, toy) => void handleCare(action, toy)}
        onOpenPage={handleOpenWorldPage}
        onSpeechControl={(control) => void handleSpeechControl(control)}
        pendingCards={legacyCompatibility ? unreadOpportunities.length : 0}
        onOpenInbox={
          legacyCompatibility ? () => selectSection("reports") : undefined
        }
        verdictReaction={verdictReaction}
      />
    </BuddyErrorBoundary>
  );

  return (
    <div className={classNames(styles.page, "rf-enter")}>
      <BuddyPageHeader
        onBack={handleBack}
        identity={{ name, statusPill, projectName, projectRoot }}
      />

      <Tabs
        value={section}
        onValueChange={(value) =>
          selectSection(value as BuddyHomeSection, false)
        }
        className={styles.sections}
      >
        <Tabs.List
          activeIndex={SECTION_ORDER.indexOf(section)}
          itemCount={SECTION_ORDER.length}
          className={styles.tabsList}
          aria-label="Buddy sections"
        >
          {SECTION_ORDER.map((value) => (
            <Tabs.Trigger key={value} value={value}>
              {SECTION_LABELS[value]}
            </Tabs.Trigger>
          ))}
        </Tabs.List>

        <main
          ref={contentRef}
          className={classNames(styles.content, styles.homeContent)}
          data-testid="buddy-home-content"
          data-section={section}
        >
          {degraded && (
            <BuddyDegradedBanner
              name={name}
              degraded={degraded}
              unavailableReason={operator.currentData?.unavailable_reason}
              isRetrying={isReopeningOperator}
              retryOutcome={operatorRetryOutcome}
              onRetry={handleRetryOperator}
            />
          )}

          <Tabs.Content value="home" className={styles.tabContent}>
            <BuddyHomeOverviewSection
              name={name}
              degraded={degraded !== null}
              world={renderWorld(false)}
              state={state}
              headline={snapshot.state.semantic.headline}
              vibe={personality?.vibe}
              heroSpeech={heroSpeech}
              onSelectSection={selectSection}
              onSpeechControl={handleSpeechControl}
              editorial={editorial}
              homeWorkMode={homeWorkMode}
              project={operator.currentData?.project ?? null}
              onOpenWork={handleOpenWork}
              onShowReports={handleShowReports}
            />
          </Tabs.Content>

          <Tabs.Content value="reports" className={styles.tabContent}>
            <BuddyReportsSection
              legacyCompatibility={legacyCompatibility}
              operatorEnabled={operatorEnabled}
              degraded={degraded !== null}
              draftId={draftId}
              mappedDraft={mappedDraft}
              deskKey={JSON.stringify([
                operator.currentData?.worker,
                mappedDraft.currentData?.data.value.id,
                selectedWork?.workId,
              ])}
              workId={selectedWork?.workId ?? pageWorkId}
              projectId={selectedWork?.projectId ?? pageProjectId}
            />
          </Tabs.Content>

          <Tabs.Content value="activity" className={styles.tabContent}>
            <BuddyActivitySection
              legacyCompatibility={legacyCompatibility}
              legacyRecentErrors={legacyRecentErrors}
              recentErrorRows={recentErrorRows}
              activities={activities}
              onInvestigateError={handleInvestigateError}
              onDismissError={handleDismissError}
              onOpenActivityChat={handleOpenActivityChat}
            />
          </Tabs.Content>

          <Tabs.Content value="play" className={styles.tabContent}>
            <BuddyPlaySection
              world={renderWorld(true)}
              degraded={degraded !== null}
              onResetWorld={handleResetWorld}
              summary={{
                name,
                palette,
                stage,
                stageIndex,
                xp,
                xpNext,
                xpFill,
                atMaxStage,
                pet,
                statsData,
                successRate,
                onViewStats: handleViewStats,
              }}
            />
          </Tabs.Content>

          <Tabs.Content value="settings" className={styles.tabContent}>
            <BuddySettingsSection
              personality={personality}
              pet={pet}
              unlockedSkills={unlockedSkills}
              activeQuest={activeQuest}
              name={name}
              settings={settings}
              degraded={degraded !== null}
              operatorNeedsRetry={operatorNeedsRetry}
              onQuestControl={handleQuestControl}
              onReroll={handleReroll}
              onRunMode={handleRunMode}
            />
          </Tabs.Content>
        </main>
      </Tabs>
    </div>
  );
};

export const BuddyHome: React.FC<BuddyHomeProps> = (props) => (
  <BuddyPresenceProvider>
    <BuddyHomeContent {...props} />
  </BuddyPresenceProvider>
);
