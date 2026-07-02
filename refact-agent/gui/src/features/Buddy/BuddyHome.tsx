import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  Flex,
  Icon,
  IconButton,
  LoadingState,
  Surface,
  Text,
} from "../../components/ui";
import { ArrowLeft, Settings, Wrench } from "lucide-react";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop, push, selectCurrentPage } from "../Pages/pagesSlice";
import { BuddyRecentChats } from "./BuddyRecentChats";
import { BuddyPulseCard } from "./BuddyPulseCard";
import { BuddyOpportunitiesFeed } from "./BuddyOpportunitiesFeed";
import { BuddySettingsPanel } from "./BuddySettingsPanel";
import { BuddyWorld } from "./BuddyWorld";
import { BuddySummaryStrip } from "./BuddySummaryStrip";
import { ArtifactsPanel } from "./ArtifactsPanel";
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
  selectHomeSnoozedUntil,
  selectSeenNotificationIds,
  dismissRuntimeEvent,
  snoozeHomeNotifications,
  markBuddyNotificationSeen,
  clearExpiredBuddyNotificationSnooze,
  setBuddySnapshot,
} from "./buddySlice";
import {
  openBuddyChat,
  openChatInModeAndStart,
  startBuddyInvestigation,
} from "../Chat/Thread";
import {
  executeBuddyAction,
  navigateFromBuddyPage,
} from "./executeBuddyAction";
import {
  buildBuddySceneSpeechCandidates,
  isBuddySpeechExpired,
  pickBuddySceneSpeechCandidate,
  type BuddySceneSpeech,
} from "./buddySceneSpeech";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
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
    await deleteDraft(draftId).unwrap();
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
          <Text size="3" weight="bold">
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
    </Surface>
  );
};

export const BuddyHome: React.FC = () => {
  const dispatch = useAppDispatch();
  const currentPage = useAppSelector(selectCurrentPage);
  const draftId =
    currentPage?.name === "buddy" ? currentPage.draftId : undefined;
  const snapshot = useAppSelector(selectBuddySnapshot);
  const loaded = useAppSelector(selectBuddyLoaded);
  const enabled = useAppSelector(selectIsBuddyInteractiveEnabled);
  const activities = useAppSelector(selectBuddyActivities);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const suggestions = useAppSelector(selectBuddySuggestions);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const pulse = useAppSelector(selectPulse);
  const unreadOpportunities = useAppSelector(selectUnreadOpportunities);
  const homeSnoozedUntil = useAppSelector(selectHomeSnoozedUntil);
  const seenNotificationIds = useAppSelector(selectSeenNotificationIds);
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();
  const executeOpportunityAction = useExecuteBuddyAction();
  const buddy = useBuddyState();
  const { state, signal: buddySignal } = buddy;
  const [setupDismissed, setSetupDismissed] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [speechIndex, setSpeechIndex] = useState(0);
  const [recentErrorNow, setRecentErrorNow] = useState(() => Date.now());
  const [isRetryingBuddySnapshot, setIsRetryingBuddySnapshot] = useState(false);
  const [updateSettings, { isLoading: isSavingSettings }] =
    useUpdateBuddySettingsMutation();

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
  const nextStage = STAGES[stageIndex + 1];
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
      nowPlaying,
      runtimeQueue,
      activeSuggestion,
      activeOpportunities: unreadOpportunities,
    }).filter((speech) => !(speech.id in seenNotificationIds));
  }, [
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
        dispatch(markBuddyNotificationSeen(heroSpeech.id));
        dispatch(snoozeHomeNotifications(undefined));
        dispatch(dismissRuntimeEvent(heroSpeech.runtimeEventId));
        void dismissRuntimeMutation(heroSpeech.runtimeEventId)
          .unwrap()
          .catch(() => undefined);
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
        dispatch(markBuddyNotificationSeen(heroSpeech.id));
        if (action.kind === "dismiss") {
          dispatch(snoozeHomeNotifications(undefined));
        }
        await executeOpportunityAction(action, opportunity, actionIndex);
        return;
      }
      if (ctrl.action === "dismiss_suggestion" && heroSpeech.suggestionId) {
        dispatch(markBuddyNotificationSeen(heroSpeech.id));
        dispatch(snoozeHomeNotifications(undefined));
      }
      await executeBuddyAction(ctrl, dispatch, {
        triggerText: heroSpeech.text,
        triggerSource:
          heroSpeech.source === "suggestion" ? "suggestion" : "runtime",
        sourceChatId: heroSpeech.chat_id,
        diagnostic: activeDiagnostic,
      });
    },
    [
      activeDiagnostic,
      dismissRuntimeMutation,
      dispatch,
      executeOpportunityAction,
      heroSpeech,
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
          <Text size="2" weight="bold" className={styles.topTitle}>
            {name}
          </Text>
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
        <Text size="2" weight="bold" className={styles.topTitle}>
          {name}
        </Text>
        <IconButton
          variant="ghost"
          size="sm"
          aria-label="Settings"
          icon={Settings}
          onClick={() => setShowSettings((v) => !v)}
        />
      </div>

      <main
        className={classNames(styles.content, styles.homeContent, "rf-stagger")}
        data-testid="buddy-home-content"
      >
        <Surface
          as="section"
          variant="glass"
          radius="card"
          animated="rise"
          className={styles.heroCard}
          data-testid="buddy-home-hero"
        >
          <BuddyWorld
            homeDoorDisabled
            palette={palette}
            stage={stage}
            state={state}
            pulse={pulse}
            pet={pet}
            nowPlaying={nowPlaying}
            activeQuest={activeQuest}
            onCanvasEvent={buddy.handleCanvasEvent}
            activeSpeech={heroSpeech}
            setupNeeded={setupNeeded}
            onRunMode={handleRunMode}
            onDismissSetup={handleDismissSetup}
            onCare={(action, toy) => void handleCare(action, toy)}
            onOpenPage={handleOpenWorldPage}
            onSpeechControl={(control) => void handleSpeechControl(control)}
          />
        </Surface>

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

        <Surface
          as="section"
          variant="glass"
          radius="card"
          animated="rise"
          className={styles.setupStrip}
          data-testid="buddy-setup-strip"
        >
          <span className={styles.setupLabel}>
            <Icon icon={Wrench} size="sm" tone="muted" />
            Project setup
          </span>
          {SETUP_MODES.map((m) => (
            <Button
              key={m.mode}
              type="button"
              size="sm"
              variant={m.mode === "setup" ? "primary" : "ghost"}
              onClick={() => handleRunMode(m.mode)}
            >
              {m.label}
            </Button>
          ))}
        </Surface>

        {showSettings && (
          <div
            className={classNames(
              styles.settingsSection,
              "rf-expand-grid",
              "rf-enter-rise",
            )}
            data-state="open"
            data-testid="buddy-home-settings-section"
          >
            <BuddySettingsPanel onClose={() => setShowSettings(false)} />
          </div>
        )}

        {draftId && (
          <div className={classNames(styles.draftSection, "rf-enter-rise")}>
            <BuddyHomeDraftReview draftId={draftId} />
          </div>
        )}

        <section
          className={classNames(styles.artifactsSection, "rf-enter-rise")}
        >
          <ArtifactsPanel />
        </section>

        <section className={classNames(styles.mainGrid, "rf-stagger")}>
          <div className={styles.panelColumn}>
            <div
              className={classNames(
                styles.panelSlot,
                styles.panelSlotOpportunities,
              )}
            >
              <BuddyOpportunitiesFeed />
            </div>
            <div
              className={classNames(
                styles.panelSlot,
                styles.panelSlotPersonality,
              )}
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
            </div>
          </div>
          <div className={styles.panelColumn}>
            <div
              className={classNames(styles.panelSlot, styles.panelSlotChats)}
            >
              <BuddyRecentChats title="Recent chats" />
            </div>
            <div
              className={classNames(styles.panelSlot, styles.panelSlotActivity)}
            >
              <BuddyActivityPanel
                activities={activities}
                onOpenChat={handleOpenActivityChat}
              />
            </div>
          </div>
          <div className={styles.panelColumn}>
            <div
              className={classNames(styles.panelSlot, styles.panelSlotPulse)}
            >
              <BuddyPulseCard />
            </div>
            <div
              className={classNames(styles.panelSlot, styles.panelSlotErrors)}
            >
              <BuddyRecentErrorsPanel
                recentErrors={recentErrors}
                onInvestigate={handleInvestigateError}
                onDismiss={handleDismissError}
              />
            </div>
          </div>
        </section>
      </main>
    </div>
  );
};
