import React, { useCallback, useMemo, useState } from "react";
import { Flex, Text, Button, Spinner, Tooltip } from "@radix-ui/themes";
import { ArrowLeftIcon, GearIcon } from "@radix-ui/react-icons";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop, push } from "../Pages/pagesSlice";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddyRecentChats } from "./BuddyRecentChats";
import { BuddyPulseCard } from "./BuddyPulseCard";
import { BuddyOpportunitiesFeed } from "./BuddyOpportunitiesFeed";
import { BuddyWorkshop } from "./BuddyWorkshop";
import { BuddySettingsPanel } from "./BuddySettingsPanel";
import { useBuddyState } from "./hooks/useBuddyState";
import {
  selectBuddySnapshot,
  selectBuddyLoaded,
  selectIsBuddyEnabled,
  selectBuddyActivities,
  selectNowPlaying,
  selectActiveSpeech,
  selectBuddyDiagnostics,
  selectRuntimeQueue,
  dismissRuntimeEvent,
} from "./buddySlice";
import {
  openChatInModeAndStart,
  startBuddyInvestigation,
} from "../Chat/Thread";
import { executeBuddyAction } from "./executeBuddyAction";
import { useDismissBuddyRuntimeEventMutation } from "../../services/refact/buddy";
import type {
  BuddyControl,
  BuddyCareAction,
  BuddyNeeds,
  BuddyRuntimeEvent,
} from "./types";
import { PALETTES, STAGES, SKILLS, SIGNALS } from "./constants";
import { computeXpFill } from "./buddyUtils";
import { useGetStatsSummaryQuery } from "../../services/refact/stats";
import { useGetSetupStatusQuery } from "../../services/refact/setupStatus";
import { SETUP_MODES } from "../Setup/setupModes";
import { useUpdateBuddySettingsMutation } from "../../services/refact/buddy";
import styles from "./BuddyHome.module.css";

const NEED_ROWS: Array<{
  key: keyof BuddyNeeds;
  label: string;
  invert?: boolean;
}> = [
  { key: "hunger", label: "Hunger" },
  { key: "energy", label: "Energy" },
  { key: "hygiene", label: "Hygiene" },
  { key: "boredom", label: "Boredom", invert: true },
  { key: "affection", label: "Affection" },
];

const CARE_ACTIONS: Array<{
  action: BuddyCareAction;
  label: string;
  emoji: string;
  toy?: string;
}> = [
  { action: "feed", label: "Feed", emoji: "🍜" },
  { action: "play", label: "Play", emoji: "🎾", toy: "bug" },
  { action: "pet", label: "Pet", emoji: "💕" },
  { action: "sleep", label: "Sleep", emoji: "😴" },
  { action: "clean", label: "Clean", emoji: "🧼" },
];

function formatTime(ts: string): string {
  if (!ts) return "";
  return new Date(ts).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export const BuddyHome: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const loaded = useAppSelector(selectBuddyLoaded);
  const enabled = useAppSelector(selectIsBuddyEnabled);
  const activities = useAppSelector(selectBuddyActivities);
  const nowPlaying = useAppSelector(selectNowPlaying);
  const activeSpeech = useAppSelector(selectActiveSpeech);
  const diagnostics = useAppSelector(selectBuddyDiagnostics);
  const runtimeQueue = useAppSelector(selectRuntimeQueue);
  const [dismissRuntimeMutation] = useDismissBuddyRuntimeEventMutation();
  const buddy = useBuddyState();
  const { state } = buddy;
  const [setupDismissed, setSetupDismissed] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
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
  const semantic = snapshot?.state.semantic;
  const pet = snapshot?.state.pet;
  const personality = snapshot?.state.personality;
  const settings = snapshot?.settings;
  const activeQuest = snapshot?.state.active_quest ?? null;

  const stage = STAGES[progression?.stage ?? state.progress.stage] ?? STAGES[0];
  const nextStage = STAGES[(progression?.stage ?? state.progress.stage) + 1];

  const xp = progression?.xp ?? state.progress.xp;
  const xpNext = progression?.xp_next ?? nextStage?.xpThreshold;
  const xpFill = useMemo(
    () => computeXpFill(progression?.xp ?? 0, progression?.xp_next ?? 100),
    [progression],
  );

  const name = identity?.name ?? state.name;
  const statusText = semantic?.headline ?? "";
  const needRows = useMemo(
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

  const handleViewStats = useCallback(() => {
    dispatch(push({ name: "stats dashboard" }));
  }, [dispatch]);

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
    [dispatch],
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

  const activeDiagnostic = activeSpeech?.chat_id
    ? diagnostics.find((diag) => diag.chat_id === activeSpeech.chat_id)
    : undefined;

  const handleSpeechControl = useCallback(
    async (ctrl: BuddyControl) => {
      if (!activeSpeech) return;
      await executeBuddyAction(ctrl, dispatch, {
        triggerText: activeSpeech.text,
        triggerSource: "runtime",
        sourceChatId: activeSpeech.chat_id,
        diagnostic: activeDiagnostic,
      });
    },
    [dispatch, activeSpeech, activeDiagnostic],
  );

  const handleQuestControl = useCallback(
    async (ctrl: BuddyControl) => {
      await executeBuddyAction(ctrl, dispatch, {
        triggerText: activeQuest?.title ?? "Buddy quest",
        triggerSource: "suggestion",
      });
    },
    [activeQuest?.title, dispatch],
  );

  const unlockedSkills = skills?.unlocked ?? state.skills;

  // All buddy-collected runtime errors, ordered newest-first. Includes
  // dismissed/acknowledged entries so the user can see the full history of
  // what Buddy has noticed. The "Investigate" button per card opens a fresh
  // Buddy chat scoped to that error, the "Dismiss" button persists the ack.
  const recentErrors = useMemo(() => {
    const items: BuddyRuntimeEvent[] = [];
    if (
      nowPlaying &&
      (nowPlaying.status === "failed" ||
        nowPlaying.priority === "critical" ||
        nowPlaying.priority === "high")
    ) {
      items.push(nowPlaying);
    }
    for (const e of runtimeQueue) {
      if (
        e.status === "failed" ||
        e.priority === "critical" ||
        e.priority === "high"
      ) {
        if (!items.find((x) => x.id === e.id)) items.push(e);
      }
    }
    items.sort((a, b) => {
      const ta = new Date(a.created_at).getTime() || 0;
      const tb = new Date(b.created_at).getTime() || 0;
      return tb - ta;
    });
    return items.slice(0, 25);
  }, [nowPlaying, runtimeQueue]);

  const handleInvestigateError = useCallback(
    (event: BuddyRuntimeEvent) => {
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
      // Investigating implicitly acknowledges the error.
      if (!event.dismissed) {
        dispatch(dismissRuntimeEvent(event.id));
        void dismissRuntimeMutation(event.id).catch(() => undefined);
      }
    },
    [dispatch, diagnostics, dismissRuntimeMutation],
  );

  const handleDismissError = useCallback(
    (event: BuddyRuntimeEvent) => {
      if (event.dismissed) return;
      dispatch(dismissRuntimeEvent(event.id));
      void dismissRuntimeMutation(event.id).catch(() => undefined);
    },
    [dispatch, dismissRuntimeMutation],
  );

  if (!loaded) {
    return (
      <div className={styles.page}>
        <Flex align="center" justify="center" style={{ flex: 1 }}>
          <Spinner size="3" />
        </Flex>
      </div>
    );
  }

  if (snapshot === null || !enabled) {
    return (
      <div className={styles.page}>
        <div className={styles.topBar}>
          <Button variant="ghost" size="1" onClick={handleBack}>
            <ArrowLeftIcon width={14} height={14} />
            Back
          </Button>
        </div>
        <Flex
          align="center"
          justify="center"
          direction="column"
          gap="2"
          style={{ flex: 1 }}
        >
          <Text size="2" color="gray">
            Buddy is not available
          </Text>
        </Flex>
      </div>
    );
  }

  return (
    <div className={styles.page}>
      <div className={styles.topBar}>
        <Button variant="ghost" size="1" onClick={handleBack}>
          <ArrowLeftIcon width={14} height={14} />
          Back
        </Button>
        <Text size="2" weight="bold" className={styles.topTitle}>
          {stage.emoji} {name}
        </Text>
        <Button
          variant="ghost"
          size="1"
          onClick={() => setShowSettings((v) => !v)}
        >
          <GearIcon width={14} height={14} />
        </Button>
      </div>

      <div className={styles.hero}>
        <div className={styles.scene}>
          <div className={styles.glowWrap}>
            <div
              className={styles.glow}
              style={{ backgroundColor: palette.body }}
            />
            <BuddyCanvas
              state={state}
              onEvent={buddy.handleCanvasEvent}
              displaySize={320}
              speechOverride={
                activeSpeech
                  ? activeSpeech.text
                  : nowPlaying?.speech_text ?? nowPlaying?.title ?? null
              }
              speechControls={activeSpeech ? activeSpeech.controls : undefined}
              onSpeechControlClick={
                activeSpeech ? handleSpeechControl : undefined
              }
            />
          </div>
        </div>

        <div
          className={styles.stageBadge}
          style={{
            backgroundColor: palette.body + "33",
            color: palette.body,
          }}
        >
          {stage.emoji} {stage.name}
        </div>

        {statusText && <div className={styles.statusText}>{statusText}</div>}

        {nowPlaying && nowPlaying.progress != null && (
          <div className={styles.statusBubble}>
            <span className={styles.statusIcon}>
              {SIGNALS[nowPlaying.signal_type]?.icon ?? "⚡"}
            </span>
            <div className={styles.statusContent}>
              <div className={styles.progressBar}>
                <div style={{ width: `${nowPlaying.progress}%` }} />
              </div>
            </div>
          </div>
        )}

        {setupNeeded && (
          <div className={styles.setupChips}>
            {SETUP_MODES.map((m) => (
              <Button
                key={m.mode}
                size="1"
                variant={m.mode === "setup" ? "soft" : "outline"}
                onClick={() => handleRunMode(m.mode)}
              >
                {m.label}
              </Button>
            ))}
            <Button
              size="1"
              variant="ghost"
              color="gray"
              onClick={handleDismissSetup}
            >
              Dismiss
            </Button>
          </div>
        )}

        <div className={styles.careBar}>
          {CARE_ACTIONS.map((item) => (
            <button
              key={item.action}
              type="button"
              className={styles.careButton}
              onClick={() => void handleCare(item.action, item.toy)}
            >
              <span>{item.emoji}</span>
              <span>{item.label}</span>
            </button>
          ))}
        </div>
      </div>

      {/* Compact summary strip — replaces the legacy STATUS card. */}
      <div className={styles.summaryStrip}>
        <div className={styles.statItem}>
          <Text size="1" color="gray">
            Stage
          </Text>
          <Text size="2" weight="bold">
            {stage.emoji} {stage.name}
          </Text>
        </div>
        <div className={classNames(styles.statItem, styles.statItemGrow)}>
          <div className={styles.statItemHeader}>
            <Text size="1" color="gray">
              Growth
            </Text>
            <Text size="1" weight="bold">
              {xp}
              {xpNext ? ` / ${xpNext}` : " (max)"}
            </Text>
          </div>
          <div className={styles.xpBar}>
            <div className={styles.xpFill} style={{ width: `${xpFill}%` }} />
          </div>
        </div>
        {pet && (
          <div className={styles.statItem}>
            <Text size="1" color="gray">
              Care
            </Text>
            <Text size="2" weight="bold">
              {pet.evolution.care_score}
            </Text>
          </div>
        )}
        {pet && (
          <div className={styles.statItem}>
            <Text size="1" color="gray">
              Neglect
            </Text>
            <Text size="2" weight="bold">
              {pet.evolution.neglect_score}
            </Text>
          </div>
        )}
        {statsData && (
          <>
            <div className={styles.statItemDivider} aria-hidden />
            <div className={styles.statItem}>
              <Text size="1" color="gray">
                Messages
              </Text>
              <Text size="2" weight="bold">
                {statsData.totals.total_calls.toLocaleString()}
              </Text>
            </div>
            <div className={styles.statItem}>
              <Text size="1" color="gray">
                Tokens
              </Text>
              <Text size="2" weight="bold">
                {(statsData.totals.total_tokens / 1000).toFixed(1)}k
              </Text>
            </div>
            <div className={styles.statItem}>
              <Text size="1" color="gray">
                Success
              </Text>
              <Text size="2" weight="bold">
                {successRate ?? 0}%
              </Text>
            </div>
          </>
        )}
        <div className={styles.statSpacer} aria-hidden />
        {statsData && (
          <Button size="1" variant="ghost" onClick={handleViewStats}>
            View Full Stats →
          </Button>
        )}
      </div>

      {showSettings && (
        <div style={{ padding: "0 var(--space-3) var(--space-3)" }}>
          <BuddySettingsPanel onClose={() => setShowSettings(false)} />
        </div>
      )}

      {/* Pulse + Opportunities + Workshop */}
      <div className={styles.row} data-testid="buddy-home-new-sections">
        <BuddyPulseCard />
        <BuddyOpportunitiesFeed />
      </div>
      <div style={{ padding: "0 var(--space-3) var(--space-3)" }}>
        <BuddyWorkshop />
      </div>

      {/* Row 1 — Care loop + Personality */}
      <div className={styles.row}>
        <div className={styles.panel}>
          <div className={styles.panelHeader}>
            <Text
              size="1"
              weight="bold"
              color="gray"
              className={styles.sectionLabel}
            >
              CARE LOOP
            </Text>
          </div>
          <div className={styles.needsGrid}>
            {needRows.map((item) => (
              <div key={item.key} className={styles.needRow}>
                <div className={styles.needHeader}>
                  <span>{item.label}</span>
                  <span>{item.value}</span>
                </div>
                <div className={styles.needBar}>
                  <div
                    className={styles.needFill}
                    style={{ width: `${item.fill}%` }}
                  />
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className={styles.panel}>
          <div className={styles.panelHeader}>
            <div className={styles.panelTitleGroup}>
              <Text
                size="1"
                weight="bold"
                color="gray"
                className={styles.sectionLabel}
              >
                PERSONALITY
              </Text>
              <Text size="2" weight="bold">
                {personality?.archetype_label ?? "Buddy"}
              </Text>
              <Text size="1" color="gray">
                {personality?.vibe ?? "Playful, quirky, helpful"}
              </Text>
            </div>
            <Button size="1" variant="soft" onClick={() => void handleReroll()}>
              Reroll
            </Button>
          </div>

          {personality?.summary && (
            <Text size="1" className={styles.personalitySummary}>
              {personality.summary}
            </Text>
          )}

          <div className={styles.traitsGrid}>
            {Object.entries(personality?.traits ?? {}).map(([key, value]) => (
              <div key={key} className={styles.traitRow}>
                <span className={styles.traitName}>{key}</span>
                <span className={styles.traitValue}>{value}</span>
              </div>
            ))}
          </div>

          <Flex direction="column" gap="1">
            <Text
              size="1"
              weight="bold"
              color="gray"
              className={styles.sectionLabel}
            >
              SKILLS
            </Text>
            <div className={styles.skillsRow}>
              {unlockedSkills.length === 0 && (
                <Text size="1" color="gray">
                  None yet
                </Text>
              )}
              {unlockedSkills.map((id) => {
                const skill = SKILLS.find((s) => s.id === id);
                return skill ? (
                  <span key={id} className={styles.skillChip}>
                    {skill.icon} {skill.name}
                  </span>
                ) : null;
              })}
            </div>
          </Flex>

          <div className={styles.settingsRow}>
            <Button
              size="1"
              variant={settings?.proactive_enabled ? "soft" : "outline"}
              onClick={handleSettings}
              disabled={isSavingSettings}
            >
              {settings?.proactive_enabled ? "Proactive On" : "Proactive Off"}
            </Button>
            <Button
              size="1"
              variant="outline"
              onClick={() =>
                void handlePromptChange(
                  settings?.personality_prompt
                    ? null
                    : personality?.prompt ?? null,
                )
              }
              disabled={isSavingSettings}
            >
              {settings?.personality_prompt
                ? "Use Random Vibe"
                : "Use Current Vibe"}
            </Button>
          </div>

          {activeQuest && (
            <div className={styles.questCard}>
              <div className={styles.questHeader}>
                <div>
                  <Text
                    size="1"
                    weight="bold"
                    color="gray"
                    className={styles.sectionLabel}
                  >
                    ACTIVE QUEST
                  </Text>
                  <Text size="2" weight="bold">
                    {activeQuest.icon} {activeQuest.title}
                  </Text>
                </div>
                <Text size="1" color="gray">
                  +{activeQuest.reward_xp} growth
                </Text>
              </div>

              <Text size="1" className={styles.questDescription}>
                {activeQuest.description}
              </Text>

              <div className={styles.questProgressRow}>
                <Text size="1" color="gray">
                  Progress
                </Text>
                <Text size="1" weight="bold">
                  {Math.min(activeQuest.progress, activeQuest.goal)} /{" "}
                  {activeQuest.goal}
                </Text>
              </div>
              <div className={styles.questProgressBar}>
                <div
                  className={styles.questProgressFill}
                  style={{
                    width: `${Math.min(
                      100,
                      (Math.max(0, activeQuest.progress) /
                        Math.max(1, activeQuest.goal)) *
                        100,
                    )}%`,
                  }}
                />
              </div>

              <div className={styles.questControls}>
                {activeQuest.controls.map((ctrl) => (
                  <Button
                    key={ctrl.id}
                    size="1"
                    variant={ctrl.style === "primary" ? "soft" : "outline"}
                    onClick={() => void handleQuestControl(ctrl)}
                  >
                    {ctrl.label}
                  </Button>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Row 2 — Project setup + Activity (activity scrolls internally) */}
      <div className={classNames(styles.row, styles.rowFlex)}>
        <div className={styles.panel}>
          <div className={styles.panelHeader}>
            <Text
              size="1"
              weight="bold"
              color="gray"
              className={styles.sectionLabel}
            >
              PROJECT SETUP
            </Text>
          </div>
          <div className={styles.setupChipsList}>
            {SETUP_MODES.map((m) => (
              <button
                key={m.mode}
                type="button"
                className={classNames(styles.setupChip, {
                  [styles.setupChipPrimary]: m.mode === "setup",
                })}
                onClick={() => handleRunMode(m.mode)}
              >
                {m.label}
              </button>
            ))}
          </div>
        </div>

        <div className={classNames(styles.panel, styles.panelScroll)}>
          <div className={styles.panelHeader}>
            <Text
              size="1"
              weight="bold"
              color="gray"
              className={styles.sectionLabel}
            >
              ACTIVITY
            </Text>
          </div>
          <div className={styles.scrollList}>
            {activities.length === 0 && (
              <Text size="1" className={styles.emptyText}>
                No recent activity
              </Text>
            )}
            {activities.map((a, i) => {
              const tooltip = a.description || a.title;
              return (
                <Tooltip
                  key={`${a.activity_type}-${a.timestamp}-${i}`}
                  content={tooltip}
                  delayDuration={150}
                >
                  <div
                    className={styles.listRow}
                    tabIndex={0}
                    role="listitem"
                    aria-label={tooltip}
                  >
                    <span className={styles.listIcon}>{a.icon}</span>
                    <div className={styles.listContent}>
                      <span className={styles.listTitle}>{a.title}</span>
                    </div>
                    <span className={styles.listMeta}>
                      {formatTime(a.timestamp)}
                    </span>
                  </div>
                </Tooltip>
              );
            })}
          </div>
        </div>
      </div>

      {/* Row 3 — Recent errors + Recent chats (both scroll internally) */}
      <div className={classNames(styles.row, styles.rowFlex)}>
        <div className={classNames(styles.panel, styles.panelScroll)}>
          <div className={styles.panelHeader}>
            <Text
              size="1"
              weight="bold"
              color="gray"
              className={styles.sectionLabel}
            >
              RECENT ERRORS
            </Text>
          </div>
          <div className={styles.scrollList}>
            {recentErrors.length === 0 && (
              <Text size="1" className={styles.emptyText}>
                No errors recorded — all clear ✨
              </Text>
            )}
            {recentErrors.map((e) => {
              const icon = e.dismissed
                ? "✅"
                : e.priority === "critical"
                  ? "🚨"
                  : "❌";
              const subtitle = [
                e.source,
                e.chat_id ? `chat ${e.chat_id.slice(0, 8)}` : null,
              ]
                .filter(Boolean)
                .join(" · ");
              return (
                <div
                  key={e.id}
                  className={classNames(
                    styles.listRow,
                    styles.errorRow,
                    e.dismissed && styles.errorRowAcknowledged,
                  )}
                >
                  <span className={styles.listIcon}>{icon}</span>
                  <div className={styles.listContent}>
                    <span className={styles.listTitle}>
                      {e.title}
                      {e.dismissed && (
                        <span className={styles.ackBadge}>acknowledged</span>
                      )}
                    </span>
                    {(e.description || subtitle) && (
                      <span className={styles.listSubtitle}>
                        {e.description ?? subtitle}
                      </span>
                    )}
                  </div>
                  <div className={styles.errorActions}>
                    <Tooltip content="Open a Buddy chat to investigate this error">
                      <Button
                        size="1"
                        variant="soft"
                        onClick={() => handleInvestigateError(e)}
                      >
                        Investigate
                      </Button>
                    </Tooltip>
                    {!e.dismissed && (
                      <Tooltip content="Mark as acknowledged">
                        <Button
                          size="1"
                          variant="ghost"
                          color="gray"
                          onClick={() => handleDismissError(e)}
                        >
                          Dismiss
                        </Button>
                      </Tooltip>
                    )}
                  </div>
                  <span className={styles.listMeta}>
                    {formatTime(e.created_at)}
                  </span>
                </div>
              );
            })}
          </div>
        </div>

        <BuddyRecentChats
          className={classNames(styles.panel, styles.panelScroll)}
          title="RECENT CHATS"
        />
      </div>
    </div>
  );
};
