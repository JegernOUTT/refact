import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Badge,
  Button,
  Flex,
  Icon,
  IconButton,
  LoadingState,
  Surface,
  Tabs,
  Text,
} from "../../components/ui";
import {
  AlertTriangle,
  ArrowLeft,
  MessageCircle,
  Settings,
  Wrench,
} from "lucide-react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop, push, selectCurrentPage } from "../Pages/pagesSlice";
import { BuddyRecentChats } from "./BuddyRecentChats";
import { BuddyPulseCard } from "./BuddyPulseCard";
import { BuddyOpportunitiesFeed } from "./BuddyOpportunitiesFeed";
import { BuddySettingsPanel } from "./BuddySettingsPanel";
import { BuddyAppliedChangesCard } from "./BuddyAppliedChangesCard";
import { BuddyWorld } from "./BuddyWorld";
import { BuddyErrorBoundary } from "./BuddyErrorBoundary";
import { BuddySummaryStrip } from "./BuddySummaryStrip";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { BuddyPersonalityPanel, type NeedRow } from "./BuddyPersonalityPanel";
import { BuddyActivityPanel } from "./BuddyActivityPanel";
import {
  BuddyRecentErrorsPanel,
  type RecentBuddyError,
} from "./BuddyRecentErrorsPanel";
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
import { BuddyDesk } from "./BuddyDesk";
import {
  buddyOperatorIdentity,
  useGetBuddyOperatorStatusQuery,
  useGetBuddyOperatorWorkQuery,
  useGetBuddyLegacyDraftWorkQuery,
  useReopenBuddyOperatorMutation,
  type OperatorUnavailableReason,
} from "../../services/refact/buddyOperator";
import { BuddyPresence } from "./BuddyPresence";
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
  useDeleteDraftMutation,
  useDismissBuddyRuntimeEventMutation,
  useGetDraftQuery,
  useUpdateBuddySettingsMutation,
} from "../../services/refact/buddy";
import type {
  BuddyCareAction,
  BuddyControl,
  BuddyDraft,
  BuddyNeeds,
  BuddyPage,
  BuddyRuntimeEvent,
  DraftKind,
} from "./types";
import { PALETTES, STAGES } from "./constants";
import { computeXpFill } from "./buddyUtils";
import { useGetStatsSummaryQuery } from "../../services/refact/stats";
import { useGetSetupStatusQuery } from "../../services/refact/setupStatus";
import { SETUP_MODES } from "../Setup/setupModes";
import { BuddyPresenceProvider } from "./BuddyPresenceProvider";
import styles from "./BuddyHome.module.css";

const NEED_ROWS: {
  key: keyof BuddyNeeds;
  label: string;
  invert?: boolean;
}[] = [
  { key: "hunger", label: "Hunger" },
  { key: "energy", label: "Energy" },
  { key: "hygiene", label: "Hygiene" },
  { key: "boredom", label: "Boredom", invert: true },
  { key: "affection", label: "Affection" },
];
const DRAFT_KIND_LABELS: Record<DraftKind, string> = {
  skill: "Skill",
  command: "Command",
  delegate: "Delegate",
  mode: "Mode",
  agents_md: "AGENTS.md",
  defaults_model: "Default Models",
  hook: "Hooks",
  pulse_report: "Pulse Report",
};

const REVIEWABLE_DRAFT_KINDS: DraftKind[] = ["agents_md", "pulse_report"];

type BuddyHomeTab =
  | "pulse"
  | "opportunities"
  | "errors"
  | "activity"
  | "play"
  | "settings";

const LEGACY_TAB_ORDER: BuddyHomeTab[] = [
  "pulse",
  "opportunities",
  "errors",
  "activity",
  "play",
  "settings",
];

const OPERATOR_TAB_ORDER: BuddyHomeTab[] = [
  "errors",
  "activity",
  "play",
  "settings",
];

const LEGACY_DEFAULT_TAB: BuddyHomeTab = "pulse";
const OPERATOR_DEFAULT_TAB: BuddyHomeTab = "activity";

const BUDDY_HOME_TAB_LABELS: Record<BuddyHomeTab, string> = {
  pulse: "Pulse",
  opportunities: "Opportunities",
  errors: "Recent errors",
  activity: "Activity",
  play: "Play",
  settings: "Settings",
};

type BuddyOperatorMode =
  | "loading"
  | "error"
  | "enabled"
  | "disabled"
  | "unavailable";

const OPERATOR_MODE_LABELS: Record<BuddyOperatorMode, string> = {
  loading: "Checking",
  error: "Broken",
  enabled: "On",
  disabled: "Local only",
  unavailable: "Not responding",
};

const OPERATOR_MODE_TONES: Record<
  BuddyOperatorMode,
  React.ComponentProps<typeof Badge>["tone"]
> = {
  loading: "muted",
  error: "danger",
  enabled: "success",
  disabled: "accent",
  unavailable: "danger",
};

type BuddyOperatorFailure = Extract<BuddyOperatorMode, "error" | "unavailable">;

const OPERATOR_FAILURE_TITLES: Record<BuddyOperatorFailure, string> = {
  error: "Buddy's work service answered with an error",
  unavailable: "Buddy's work service is not responding",
};

const OPERATOR_FAILURE_DETAIL: Record<BuddyOperatorFailure, string> = {
  error:
    "The engine replied, but not with anything this page can trust — so this is a connection failure, not an empty queue.",
  unavailable:
    "Nothing answered for saved work, proposals or pulse reports — so this is a connection failure, not an empty queue.",
};

const OPERATOR_FAILURE_SCOPE =
  "Your chats, files and recent warnings are untouched. Progress, counters and project setup stay hidden until something can confirm them.";

const OPERATOR_UNAVAILABLE_REASON_TITLES: Record<string, string> = {
  already_owned: "Another Refact process is using saved work",
  io_error: "Buddy could not open saved work",
  project_mismatch: "Saved work belongs to a different project",
  not_attempted: "Buddy has not opened saved work yet",
};

function unavailableReasonTitle(
  reason: OperatorUnavailableReason | undefined,
): string {
  if (!reason) return OPERATOR_FAILURE_TITLES.unavailable;
  return (
    OPERATOR_UNAVAILABLE_REASON_TITLES[reason.kind] ??
    "Buddy could not open saved work"
  );
}

function retryScheduleCopy(retryInSeconds: number | undefined): string | null {
  if (retryInSeconds === undefined) return null;
  if (retryInSeconds < 60)
    return "Buddy is already trying again in under a minute.";
  const minutes = Math.ceil(retryInSeconds / 60);
  return `Buddy is already trying again in about ${minutes} minute${
    minutes === 1 ? "" : "s"
  }.`;
}

type ClipboardWriter = {
  clipboard?: {
    writeText?: (text: string) => Promise<void>;
  };
};

const RECENT_ERROR_WINDOW_MS = 6 * 60 * 60 * 1000;
const RECENT_ERROR_REFRESH_MS = 60 * 1000;

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

function draftKindLabel(draft: BuddyDraft): string {
  return DRAFT_KIND_LABELS[draft.kind];
}

const BuddyHomeDraftReview: React.FC<{ draftId: string }> = ({ draftId }) => {
  const { data: draft, isLoading, isError } = useGetDraftQuery(draftId);
  const [deleteDraft, { isLoading: isDeleting }] = useDeleteDraftMutation();
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);
  const [dismissError, setDismissError] = useState(false);

  const handleCopy = useCallback(async () => {
    if (!draft) return;
    setCopyError(null);
    try {
      const clipboard = (navigator as ClipboardWriter).clipboard;
      if (!clipboard?.writeText) {
        throw new Error("Clipboard API unavailable");
      }
      await clipboard.writeText(draft.yaml_or_json);
      setCopied(true);
    } catch {
      setCopied(false);
      setCopyError(
        "Clipboard unavailable — select the draft text and copy it manually.",
      );
    }
  }, [draft]);

  const handleDismiss = useCallback(async () => {
    setDismissError(false);
    try {
      await deleteDraft(draftId).unwrap();
    } catch {
      setDismissError(true);
    }
  }, [deleteDraft, draftId]);

  if (isLoading) {
    return (
      <Surface className={styles.draftPanel} radius="card" variant="glass">
        <LoadingState label="Loading Buddy draft…" variant="compact" />
      </Surface>
    );
  }

  if (isError || !draft) {
    return (
      <Surface className={styles.draftPanel} radius="card" variant="glass">
        <Text size="2" color="red">
          Draft unavailable or expired.
        </Text>
      </Surface>
    );
  }

  const reviewable = REVIEWABLE_DRAFT_KINDS.includes(draft.kind);

  return (
    <Surface className={styles.draftPanel} radius="card" variant="glass">
      <div className={styles.draftHeader}>
        <div className={styles.draftTitleGroup}>
          <Text size="1" color="gray" className={styles.draftLabel}>
            Buddy draft review
          </Text>
          <Text as="h2" size="3" weight="bold">
            {draft.title}
          </Text>
        </div>
        <Text size="1" color="gray" className={styles.draftMeta}>
          {draftKindLabel(draft)} · {draft.id}
        </Text>
      </div>
      {draft.explanation && (
        <Text size="2" color="gray">
          {draft.explanation}
        </Text>
      )}
      {!reviewable && (
        <Text size="2" color="orange">
          This draft opens in its dedicated editor from the opportunity action.
        </Text>
      )}
      <pre className={styles.draftContent}>{draft.yaml_or_json}</pre>
      <div className={styles.draftActions}>
        <Button
          type="button"
          size="sm"
          variant="primary"
          onClick={() => void handleCopy()}
        >
          {copied ? "Copied" : "Copy content"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={isDeleting}
          onClick={() => void handleDismiss()}
        >
          Dismiss draft
        </Button>
      </div>
      {copyError && (
        <Text size="1" color="orange">
          {copyError}
        </Text>
      )}
      {dismissError && (
        <Text size="1" color="red" role="alert">
          Draft was not dismissed. Try again.
        </Text>
      )}
    </Surface>
  );
};

const BuddyHomeContent: React.FC = () => {
  const dispatch = useAppDispatch();
  const currentPage = useAppSelector(selectCurrentPage);
  const draftId =
    currentPage?.name === "buddy" ? currentPage.draftId : undefined;
  const snapshot = useAppSelector(selectBuddySnapshot);
  const operatorIdentity = useAppSelector(buddyOperatorIdentity);
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
    <Badge
      size={degraded ? "md" : "xs"}
      variant={degraded ? "outline" : "soft"}
      tone={OPERATOR_MODE_TONES[operatorMode]}
      className={classNames(
        styles.statusPill,
        degraded && styles.statusPillAlert,
      )}
      data-testid="buddy-operator-status"
      data-state={operatorMode}
    >
      {degraded && <Icon icon={AlertTriangle} size="sm" tone="danger" />}
      {OPERATOR_MODE_LABELS[operatorMode]}
    </Badge>
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
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
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
  const [chosenTab, setChosenTab] = useState<BuddyHomeTab | null>(null);
  const [sceneOpen, setSceneOpen] = useState(true);
  // The scene is decoration with a thumbnail's worth of decision value, so it
  // starts as a compact strip and the full diorama is opt-in. `sceneOpen` still
  // owns mount/unmount ("Open Play" / "Close Play"); this only owns size.
  const [sceneExpanded, setSceneExpanded] = useState(false);
  const legacyTabs = legacyCompatibility || operatorMode === "loading";
  const defaultTab = legacyTabs ? LEGACY_DEFAULT_TAB : OPERATOR_DEFAULT_TAB;
  const tabOrder = useMemo(() => {
    const order = legacyTabs ? LEGACY_TAB_ORDER : OPERATOR_TAB_ORDER;
    return sceneOpen ? order : order.filter((tab) => tab !== "play");
  }, [legacyTabs, sceneOpen]);
  const activeTab =
    chosenTab !== null && tabOrder.includes(chosenTab) ? chosenTab : defaultTab;
  const deskWork = useGetBuddyOperatorWorkQuery(
    {
      cursor: undefined,
      limit: 50,
      endpoint_identity: operatorIdentity,
      view: "needs_you",
      search: "",
      attention: "all",
    },
    { skip: !operatorEnabled, refetchOnMountOrArgChange: true },
  );
  const deskHasWork = (deskWork.currentData?.data.length ?? 0) > 0;
  const contentRef = useRef<HTMLElement>(null);
  const playPanelRef = useRef<HTMLDivElement>(null);
  const presenceRef = useRef<HTMLDivElement>(null);
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

  // Play is the one place the scene IS the content, so it always gets the full
  // diorama regardless of the strip preference the rest of the page uses.
  const sceneStrip = !sceneExpanded && activeTab !== "play";

  useEffect(() => {
    if (activeTab !== "play") return;
    setSceneOpen(true);
    if (contentRef.current) contentRef.current.scrollTop = 0;
    playPanelRef.current?.focus({ preventScroll: true });
  }, [activeTab]);

  useEffect(() => {
    if (!operatorNeedsRetry) return;
    setSceneOpen(false);
  }, [operatorNeedsRetry]);

  const handleToggleScene = useCallback(() => {
    if (sceneOpen && activeTab === "play") setChosenTab(null);
    setSceneOpen(!sceneOpen);
  }, [activeTab, sceneOpen]);

  const handleToggleSceneSize = useCallback(() => {
    if (activeTab === "play") setChosenTab(null);
    setSceneExpanded((expanded) => !expanded);
  }, [activeTab]);

  const handleLeavePlay = useCallback(() => {
    setChosenTab(null);
    if (contentRef.current) contentRef.current.scrollTop = 0;
    presenceRef.current?.focus({ preventScroll: true });
  }, []);

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
  const needRows = useMemo<NeedRow[]>(
    () =>
      NEED_ROWS.map((item) => {
        const value = pet?.needs[item.key] ?? 0;
        const fill = item.invert ? 100 - value : value;
        return {
          ...item,
          value,
          fill: Math.max(0, Math.min(100, fill)),
        };
      }),
    [pet],
  );

  const successRate = useMemo(() => {
    if (!statsData || statsData.totals.total_calls === 0) return null;
    return Math.round(
      (statsData.totals.successful_calls / statsData.totals.total_calls) * 100,
    );
  }, [statsData]);

  const handleBack = useCallback(() => {
    dispatch(pop());
  }, [dispatch]);

  const handleSettings = useCallback(() => {
    void updateSettings({ proactive_enabled: !settings?.proactive_enabled });
  }, [settings?.proactive_enabled, updateSettings]);

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

  const handlePromptChange = useCallback(
    async (prompt: string | null) => {
      if (prompt === null) {
        await updateSettings({ clear_personality_prompt: true });
        return;
      }
      await updateSettings({ personality_prompt: prompt });
    },
    [updateSettings],
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
    ? diagnostics.find((diag) => diag.chat_id === heroSpeech.chat_id)
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

  const recentErrors = useMemo<RecentBuddyError[]>(() => {
    const collected: BuddyRuntimeEvent[] = [];
    if (
      nowPlaying &&
      (nowPlaying.status === "failed" ||
        nowPlaying.priority === "critical" ||
        nowPlaying.priority === "high")
    ) {
      collected.push(nowPlaying);
    }
    for (const e of runtimeQueue) {
      if (
        e.status === "failed" ||
        e.priority === "critical" ||
        e.priority === "high"
      ) {
        if (!collected.find((x) => x.id === e.id)) collected.push(e);
      }
    }
    collected.sort((a, b) => {
      const ta = new Date(a.created_at).getTime() || 0;
      const tb = new Date(b.created_at).getTime() || 0;
      return tb - ta;
    });

    const cutoff = recentErrorNow - RECENT_ERROR_WINDOW_MS;
    const sigMap = new Map<string, RecentBuddyError>();
    for (const e of collected) {
      const createdAt = Date.parse(e.created_at);
      if (
        Number.isFinite(createdAt) &&
        createdAt < cutoff &&
        e.status !== "started" &&
        e.status !== "progress" &&
        e.status !== "streaming"
      ) {
        continue;
      }
      const sig = `${e.source}|${e.signal_type}|${e.title}|${
        e.description ?? ""
      }|${e.failure_category ?? ""}|${e.failure_summary ?? ""}`;
      const existing = sigMap.get(sig);
      if (existing) {
        const existingCreatedAt = Date.parse(existing.created_at);
        const incomingIsNewer =
          Number.isFinite(createdAt) &&
          (!Number.isFinite(existingCreatedAt) ||
            createdAt > existingCreatedAt);
        existing.occurrences = (existing.occurrences ?? 1) + 1;
        existing.dismissedAny =
          Boolean(existing.dismissedAny) || Boolean(e.dismissed);
        existing.dismissedAll =
          Boolean(existing.dismissedAll) && Boolean(e.dismissed);
        existing.relatedIds = Array.from(
          new Set([...(existing.relatedIds ?? [existing.id]), e.id]),
        );
        existing.failure_category =
          e.failure_category ?? existing.failure_category;
        existing.failure_summary =
          e.failure_summary ?? existing.failure_summary;
        if (incomingIsNewer) {
          existing.description = e.description ?? existing.description;
          existing.chat_id = e.chat_id ?? existing.chat_id;
          existing.created_at = e.created_at;
        }
        existing.dismissed =
          Boolean(existing.dismissed) || Boolean(e.dismissed);
      } else {
        sigMap.set(sig, {
          ...e,
          occurrences: 1,
          dismissedAny: Boolean(e.dismissed),
          dismissedAll: Boolean(e.dismissed),
          relatedIds: [e.id],
        });
      }
    }
    return Array.from(sigMap.values()).slice(0, 25);
  }, [nowPlaying, recentErrorNow, runtimeQueue]);

  const handleInvestigateError = useCallback(
    (event: RecentBuddyError) => {
      const triggerText = event.description
        ? `${event.title}: ${event.description}`
        : event.title;
      const diagnostic =
        event.chat_id != null
          ? diagnostics.find((d) => d.chat_id === event.chat_id) ?? null
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
    [dispatch, diagnostics, dismissRuntimeMutation],
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
        <div className={styles.topBar}>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={ArrowLeft}
            onClick={handleBack}
          >
            Back
          </Button>
        </div>
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
        <div className={styles.topBar}>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={ArrowLeft}
            onClick={handleBack}
          >
            Back
          </Button>
          <div className={styles.topIdentity}>
            <Text as="h1" size="5" weight="bold" className={styles.topTitle}>
              {name}
            </Text>
            {statusPill}
          </div>
          <IconButton
            variant="ghost"
            size="sm"
            aria-label="Settings"
            icon={Settings}
            onClick={() => setShowSettings((v) => !v)}
          />
        </div>
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

  return (
    <div className={classNames(styles.page, "rf-enter")}>
      <div className={styles.topBar}>
        <Button
          variant="ghost"
          size="sm"
          leftIcon={ArrowLeft}
          onClick={handleBack}
        >
          Back
        </Button>
        <div className={styles.topIdentity}>
          <Text as="h1" size="5" weight="bold" className={styles.topTitle}>
            {name}
          </Text>
          {statusPill}
        </div>
        <span className={styles.topBarSpacer} aria-hidden />
      </div>

      <main
        ref={contentRef}
        className={classNames(styles.content, styles.homeContent, "rf-stagger")}
        data-testid="buddy-home-content"
      >
        {degraded && (
          <Surface
            as="section"
            variant="glass"
            radius="card"
            className={styles.degradedCard}
            role="alert"
            data-testid="buddy-operator-degraded"
          >
            <div className={styles.degradedHeader}>
              <Icon icon={AlertTriangle} size="sm" tone="danger" />
              <Text size="3" weight="bold" className={styles.degradedTitle}>
                {degraded === "unavailable"
                  ? unavailableReasonTitle(
                      operator.currentData?.unavailable_reason,
                    )
                  : OPERATOR_FAILURE_TITLES[degraded]}
              </Text>
            </div>
            <Text size="2" className={styles.degradedText}>
              {degraded === "unavailable" &&
              operator.currentData?.unavailable_reason
                ? operator.currentData.unavailable_reason.detail
                : OPERATOR_FAILURE_DETAIL[degraded]}
            </Text>
            {degraded === "unavailable" &&
              retryScheduleCopy(
                operator.currentData?.unavailable_reason?.retry_in_seconds,
              ) && (
                <Text size="2" color="gray" className={styles.degradedText}>
                  {retryScheduleCopy(
                    operator.currentData?.unavailable_reason?.retry_in_seconds,
                  )}{" "}
                  This button tries now.
                </Text>
              )}
            <Text size="2" color="gray" className={styles.degradedText}>
              {OPERATOR_FAILURE_SCOPE}
            </Text>
            <div className={styles.degradedActions}>
              <Button
                size="sm"
                variant="primary"
                onClick={() => void handleRetryOperator()}
                disabled={isReopeningOperator}
              >
                {isReopeningOperator
                  ? "Retrying work authority…"
                  : !operator.currentData?.unavailable_reason
                    ? "Check work authority"
                    : "Retry work authority"}
              </Button>
            </div>
            {operatorRetryOutcome && (
              <Text size="2" role="status" className={styles.degradedText}>
                {operatorRetryOutcome === "recovered"
                  ? "Saved work is available again."
                  : operatorRetryOutcome === "unavailable"
                    ? "Saved work is still unavailable. Buddy will keep trying."
                    : "Buddy could not try saved work. Check work authority again later."}
              </Text>
            )}
          </Surface>
        )}

        {operatorEnabled && (
          <BuddyDesk
            key={JSON.stringify([
              operator.currentData?.worker,
              mappedDraft.currentData?.data.value.id,
            ])}
            workId={
              currentPage?.name === "buddy"
                ? currentPage.workId ?? mappedDraft.currentData?.data.value.id
                : undefined
            }
            projectId={
              currentPage?.name === "buddy"
                ? currentPage.workProjectId ??
                  mappedDraft.currentData?.project.project_id
                : undefined
            }
          />
        )}

        <BuddyNativeDecisions />
        <BuddyNativeApprovals />
        <BuddySupervision />

        {/* Prominence, not deletion: the scene follows the Desk and the
         * decision panels, and renders as a ~56px strip until someone asks for
         * the diorama. A security finding and a snack request are no longer
         * adjacent, and seasonal poetry no longer separates the work rows from
         * the controls that act on them. */}
        <Surface
          as="section"
          variant="plain"
          radius="none"
          className={classNames(
            styles.sceneBlock,
            sceneStrip && styles.sceneBlockStrip,
          )}
          aria-label="Buddy scene"
          data-testid="buddy-home-scene"
          data-scene-view={sceneStrip ? "strip" : "scene"}
        >
          {sceneOpen && (
            <Surface
              as="section"
              variant="glass"
              radius="card"
              className={classNames(
                styles.heroCard,
                sceneStrip && styles.heroCardStrip,
                activeTab === "play" && styles.heroCardExpanded,
                deskHasWork && activeTab !== "play" && styles.heroCardDemoted,
              )}
              id="buddy-home-hero"
              data-testid="buddy-home-hero"
            >
              <BuddyErrorBoundary key={worldBoundaryKey}>
                <BuddyWorld
                  homeDoorDisabled
                  compact={activeTab !== "play"}
                  palette={palette}
                  stage={stage}
                  state={state}
                  pulse={pulse}
                  pet={pet}
                  nowPlaying={nowPlaying}
                  activeQuest={activeQuest}
                  onCanvasEvent={buddy.handleCanvasEvent}
                  activeSpeech={activeTab === "play" ? heroSpeech : null}
                  setupNeeded={setupNeeded}
                  onRunMode={handleRunMode}
                  onDismissSetup={handleDismissSetup}
                  onCare={(action, toy) => void handleCare(action, toy)}
                  onOpenPage={handleOpenWorldPage}
                  onSpeechControl={(control) =>
                    void handleSpeechControl(control)
                  }
                  pendingCards={
                    legacyCompatibility ? unreadOpportunities.length : 0
                  }
                  onOpenInbox={
                    legacyCompatibility
                      ? () => setChosenTab("opportunities")
                      : undefined
                  }
                  verdictReaction={verdictReaction}
                />
              </BuddyErrorBoundary>
            </Surface>
          )}

          <div ref={presenceRef} tabIndex={-1} className={styles.presenceSlot}>
            <BuddyPresence
              state={state}
              headline={snapshot.state.semantic.headline}
              speech={activeTab === "play" ? null : heroSpeech}
              sceneMounted={sceneOpen}
              sceneToggleLabel={sceneOpen ? "Close Play" : "Open Play"}
              onToggleScene={handleToggleScene}
              onControl={handleSpeechControl}
            />
          </div>

          {sceneOpen && (
            <div className={styles.sceneSizeActions}>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                aria-expanded={!sceneStrip}
                aria-controls="buddy-home-hero"
                onClick={handleToggleSceneSize}
              >
                {sceneStrip ? "Expand scene" : "Collapse scene"}
              </Button>
            </div>
          )}
        </Surface>

        {editorial.canFeedback && editorial.speech && (
          <Surface
            as="section"
            variant="glass"
            radius="card"
            className={styles.editorialCard}
            aria-label="Buddy editorial message"
          >
            <BuddySectionHeader icon={MessageCircle} label={`${name} says`} />
            <Text size="2" className={styles.editorialText}>
              {editorial.speech.text}
            </Text>
            <div className={styles.editorialActions}>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void editorial.sendFeedback("useful")}
              >
                Helpful
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void editorial.sendFeedback("noisy")}
              >
                Noisy
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void editorial.sendFeedback("less_like_this")}
              >
                Less like this
              </Button>
            </div>
            {editorial.feedbackStatus && (
              <Text
                size="1"
                color="gray"
                role="status"
                className={styles.editorialStatus}
              >
                {editorial.feedbackStatus}
              </Text>
            )}
          </Surface>
        )}

        <Tabs
          value={activeTab}
          onValueChange={(value) => setChosenTab(value as BuddyHomeTab)}
          className={styles.tabs}
        >
          <Tabs.List
            activeIndex={tabOrder.indexOf(activeTab)}
            itemCount={tabOrder.length}
            className={styles.tabsList}
          >
            {tabOrder.map((tab) => (
              <Tabs.Trigger key={tab} value={tab}>
                {BUDDY_HOME_TAB_LABELS[tab]}
              </Tabs.Trigger>
            ))}
          </Tabs.List>

          {legacyCompatibility && (
            <Tabs.Content value="pulse" className={styles.tabContent}>
              <div className={styles.tabPanel}>
                <BuddyPulseCard />
              </div>
            </Tabs.Content>
          )}

          {legacyCompatibility && (
            <Tabs.Content value="opportunities" className={styles.tabContent}>
              <div className={styles.tabPanel}>
                <BuddyOpportunitiesFeed />
              </div>
            </Tabs.Content>
          )}

          <Tabs.Content value="errors" className={styles.tabContent}>
            {legacyCompatibility ? (
              <div className={styles.tabPanel}>
                <BuddyRecentErrorsPanel
                  recentErrors={recentErrors}
                  onInvestigate={handleInvestigateError}
                  onDismiss={handleDismissError}
                />
              </div>
            ) : (
              <div className={styles.tabNoticeList}>
                <Text size="2" color="gray" className={styles.tabNotice}>
                  {recentErrors.length === 0
                    ? "No warnings in the last six hours."
                    : "Recent warnings, read-only."}
                </Text>
                {recentErrors.map((event) => (
                  <Text
                    key={event.id}
                    size="1"
                    color="gray"
                    className={styles.tabNoticeRow}
                  >
                    {event.title}: {event.failure_summary ?? event.description}
                  </Text>
                ))}
              </div>
            )}
          </Tabs.Content>

          <Tabs.Content value="activity" className={styles.tabContent}>
            {legacyCompatibility ? (
              <div className={styles.tabPanelSplit}>
                <BuddyActivityPanel
                  activities={activities}
                  onOpenChat={handleOpenActivityChat}
                />
                <BuddyRecentChats title="Recent chats" />
              </div>
            ) : (
              <div className={styles.tabPanel}>
                <BuddyRecentChats title="Recent chats" />
              </div>
            )}
          </Tabs.Content>

          {sceneOpen && (
            <Tabs.Content value="play" className={styles.tabContent}>
              <div
                ref={playPanelRef}
                tabIndex={-1}
                aria-label="Buddy Play"
                className={styles.playPanel}
              >
                <div className={styles.playActions}>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={handleResetWorld}
                    aria-label="Reset Buddy scene"
                  >
                    Reset scene
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={handleLeavePlay}
                  >
                    Leave Play
                  </Button>
                </div>
                <Text size="1" color="gray" className={styles.playHint}>
                  Games run in the scene above.
                </Text>
              </div>
            </Tabs.Content>
          )}

          <Tabs.Content value="settings" className={styles.tabContent}>
            <div
              className={classNames(
                styles.settingsSection,
                styles.settingsStack,
              )}
              data-testid="buddy-home-settings-section"
            >
              <BuddyPersonalityPanel
                personality={personality}
                needRows={needRows}
                unlockedSkills={unlockedSkills}
                activeQuest={activeQuest}
                name={name}
                settings={settings}
                isSavingSettings={isSavingSettings}
                onQuestControl={(control) => void handleQuestControl(control)}
                onReroll={() => void handleReroll()}
                onToggleProactive={handleSettings}
                onPromptChange={(prompt) => void handlePromptChange(prompt)}
              />
              <BuddyAppliedChangesCard />
              <BuddySettingsPanel />
            </div>
          </Tabs.Content>
        </Tabs>

        {!degraded && (
          <BuddySummaryStrip
            name={name}
            palette={palette}
            stage={stage}
            stageIndex={stageIndex}
            xp={xp}
            xpNext={xpNext}
            xpFill={xpFill}
            atMaxStage={atMaxStage}
            pet={pet}
            statsData={statsData}
            successRate={successRate}
            onViewStats={handleViewStats}
          />
        )}

        <Surface
          as="section"
          variant="glass"
          radius="card"
          animated="rise"
          className={styles.setupStrip}
          data-testid="buddy-setup-strip"
          data-disabled={degraded ? "true" : undefined}
        >
          <Text as="h2" size="3" weight="bold" className={styles.setupLabel}>
            <Icon icon={Wrench} size="sm" tone="muted" />
            Project setup
          </Text>
          {SETUP_MODES.map((m) => (
            <Button
              key={m.mode}
              type="button"
              size="sm"
              variant={m.mode === "setup" ? "primary" : "ghost"}
              disabled={operatorNeedsRetry}
              onClick={() => handleRunMode(m.mode)}
            >
              {m.label}
            </Button>
          ))}
          {degraded && (
            <Text size="1" color="gray" className={styles.setupNotice}>
              Setup runs need the engine that just stopped answering.
            </Text>
          )}
        </Surface>

        {legacyCompatibility && draftId && (
          <div className={classNames(styles.draftSection, "rf-enter-rise")}>
            <BuddyHomeDraftReview draftId={draftId} />
          </div>
        )}

        {!legacyCompatibility && draftId && (
          <Surface
            as="section"
            variant="glass"
            radius="card"
            className={styles.draftNotice}
          >
            <Text size="2" color="gray" className={styles.draftNoticeText}>
              Draft editing sessions may expire. Saved proposals stay below.
            </Text>
            {mappedDraft.isFetching && (
              <Text
                size="1"
                color="gray"
                role="status"
                className={styles.draftNoticeText}
              >
                Finding the saved proposal…
              </Text>
            )}
            {mappedDraft.error && (
              <Text
                size="1"
                color="red"
                role="alert"
                className={styles.draftNoticeText}
              >
                No saved proposal could be loaded for this editing session.
              </Text>
            )}
          </Surface>
        )}
      </main>
    </div>
  );
};

export const BuddyHome: React.FC = () => (
  <BuddyPresenceProvider>
    <BuddyHomeContent />
  </BuddyPresenceProvider>
);
