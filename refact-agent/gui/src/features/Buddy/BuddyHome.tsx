import React, { useCallback, useMemo, useState } from "react";
import { Flex, Text, Button, Spinner } from "@radix-ui/themes";
import { ArrowLeftIcon, GearIcon } from "@radix-ui/react-icons";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop, push } from "../Pages/pagesSlice";
import { BuddyCanvas } from "./BuddyCanvas";
import { BuddyRecentChats } from "./BuddyRecentChats";
import { useBuddyState } from "./hooks/useBuddyState";
import {
  selectBuddySnapshot,
  selectBuddyActivities,
} from "./buddySlice";
import { PALETTES, STAGES, SKILLS } from "./constants";
import { computeXpFill } from "./buddyUtils";
import { useGetStatsSummaryQuery } from "../../services/refact/stats";
import { useGetSetupStatusQuery } from "../../services/refact/setupStatus";
import { openChatInModeAndStart } from "../Chat/Thread/actions";
import styles from "./BuddyHome.module.css";

export const BuddyHome: React.FC = () => {
  const dispatch = useAppDispatch();
  const snapshot = useAppSelector(selectBuddySnapshot);
  const activities = useAppSelector(selectBuddyActivities);
  const buddy = useBuddyState();
  const { state } = buddy;
  const [setupDismissed, setSetupDismissed] = useState(false);

  const { data: statsData } = useGetStatsSummaryQuery({});
  const { data: setupData } = useGetSetupStatusQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const setupNeeded = !setupData?.configured && !setupDismissed;

  const paletteIndex = snapshot?.settings.palette_index ?? state.paletteIndex;
  const palette = PALETTES[paletteIndex] ?? PALETTES[0];

  const progression = snapshot?.state.progression;
  const identity = snapshot?.state.identity;
  const skills = snapshot?.state.skills;
  const semantic = snapshot?.state.semantic;

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

  const handleBack = useCallback(() => {
    dispatch(pop());
  }, [dispatch]);

  const handleSettings = useCallback(() => {
    dispatch(push({ name: "customization" }));
  }, [dispatch]);

  const handleViewStats = useCallback(() => {
    dispatch(push({ name: "stats dashboard" }));
  }, [dispatch]);

  const handleRunSetup = useCallback(() => {
    void dispatch(openChatInModeAndStart({ mode: "setup" }));
  }, [dispatch]);

  const handleDismissSetup = useCallback(() => {
    setSetupDismissed(true);
  }, []);

  const unlockedSkills = skills?.unlocked ?? state.skills;

  if (snapshot === null) {
    return (
      <div className={styles.page}>
        <Flex align="center" justify="center" style={{ flex: 1 }}>
          <Spinner size="3" />
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
        <Button variant="ghost" size="1" onClick={handleSettings}>
          <GearIcon width={14} height={14} />
        </Button>
      </div>

      <div className={styles.hero}>
        <div className={styles.glowWrap}>
          <div
            className={styles.glow}
            style={{ backgroundColor: palette.body }}
          />
          <BuddyCanvas
            state={state}
            onEvent={buddy.handleCanvasEvent}
            style={{ width: 340, height: 340 }}
          />
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

        {statusText && (
          <div className={styles.statusText}>{statusText}</div>
        )}

        {setupNeeded && (
          <div className={styles.setupChips}>
            <Button size="1" variant="soft" onClick={handleRunSetup}>
              ⚙ Run Setup
            </Button>
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
      </div>

      {statsData && (
        <div className={styles.statsSummary}>
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
              {statsData.totals.total_calls > 0
                ? Math.round(
                    (statsData.totals.successful_calls /
                      statsData.totals.total_calls) *
                      100,
                  )
                : 0}
              %
            </Text>
          </div>
          <Button size="1" variant="ghost" onClick={handleViewStats}>
            View Full Stats →
          </Button>
        </div>
      )}

      <div className={styles.infoGrid}>
        <div className={styles.infoPanel}>
          <Text
            size="1"
            weight="bold"
            color="gray"
            className={styles.sectionLabel}
          >
            STATUS
          </Text>
          <Flex direction="column" gap="1">
            <Flex justify="between">
              <Text size="1" color="gray">
                Stage
              </Text>
              <Text size="1" weight="bold">
                {stage.name}
              </Text>
            </Flex>
            <Flex justify="between">
              <Text size="1" color="gray">
                XP
              </Text>
              <Text size="1" weight="bold">
                {xp} {xpNext ? `/ ${xpNext}` : "(max)"}
              </Text>
            </Flex>
          </Flex>
          <div className={styles.xpBar}>
            <div
              className={styles.xpFill}
              style={{ width: `${xpFill}%` }}
            />
          </div>
          <Text
            size="1"
            weight="bold"
            color="gray"
            className={styles.sectionLabel}
            style={{ marginTop: "var(--space-1)" }}
          >
            SKILLS
          </Text>
          <Flex wrap="wrap" gap="1">
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
          </Flex>
          {semantic?.last_active && (
            <Flex justify="between">
              <Text size="1" color="gray">
                Last active
              </Text>
              <Text size="1">
                {new Date(semantic.last_active).toLocaleDateString()}
              </Text>
            </Flex>
          )}
        </div>

        <div className={styles.infoPanel}>
          <Text
            size="1"
            weight="bold"
            color="gray"
            className={styles.sectionLabel}
          >
            ACTIVITY
          </Text>
          {activities.length === 0 && (
            <Text size="1" color="gray">
              No recent activity
            </Text>
          )}
          {activities.slice(0, 6).map((a, i) => (
            <div key={i} className={styles.activityItem}>
              <span className={styles.activityIcon}>{a.icon}</span>
              <span className={styles.activityDesc}>{a.title}</span>
              <span className={styles.activityTime}>
                {a.timestamp
                  ? new Date(a.timestamp).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })
                  : ""}
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className={styles.chatsSection}>
        <BuddyRecentChats />
      </div>
    </div>
  );
};
