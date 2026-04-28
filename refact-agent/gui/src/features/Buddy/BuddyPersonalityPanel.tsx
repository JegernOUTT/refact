import React from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import { SKILLS } from "./constants";
import type {
  BuddyControl,
  BuddyNeeds,
  BuddyPersonalityProfile,
  BuddyQuest,
  BuddySettings,
} from "./types";
import styles from "./BuddyHome.module.css";

export interface NeedRow {
  key: keyof BuddyNeeds;
  label: string;
  value: number;
  fill: number;
  invert?: boolean;
}

interface BuddyPersonalityPanelProps {
  personality: BuddyPersonalityProfile | undefined;
  needRows: NeedRow[];
  unlockedSkills: string[];
  activeQuest: BuddyQuest | null;
  settings: BuddySettings | undefined;
  isSavingSettings: boolean;
  onQuestControl: (control: BuddyControl) => void;
  onReroll: () => void;
  onToggleProactive: () => void;
  onPromptChange: (prompt: string | null) => void;
}

export const BuddyPersonalityPanel: React.FC<BuddyPersonalityPanelProps> = ({
  personality,
  needRows,
  unlockedSkills,
  activeQuest,
  settings,
  isSavingSettings,
  onQuestControl,
  onReroll,
  onToggleProactive,
  onPromptChange,
}) => (
  <div
    className={classNames(styles.row, styles.rowSingle)}
    data-testid="buddy-personality-panel"
  >
    <div className={classNames(styles.panel, styles.personaPanel)}>
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
      </div>

      {personality?.summary && (
        <Text size="1" className={styles.personalitySummary}>
          {personality.summary}
        </Text>
      )}

      <div className={styles.personaGrid}>
        <div className={styles.personaSection}>
          <Text
            size="1"
            weight="bold"
            color="gray"
            className={styles.sectionLabel}
          >
            NEEDS
          </Text>
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

        <div className={styles.personaSection}>
          <Text
            size="1"
            weight="bold"
            color="gray"
            className={styles.sectionLabel}
          >
            TRAITS
          </Text>
          <div className={styles.traitsGrid}>
            {Object.entries(personality?.traits ?? {}).map(([key, value]) => {
              const fill = Math.max(0, Math.min(100, Number(value) || 0));
              return (
                <div key={key} className={styles.traitRow}>
                  <div className={styles.traitHeader}>
                    <span className={styles.traitName}>{key}</span>
                    <span className={styles.traitValue}>{value}</span>
                  </div>
                  <div className={styles.needBar}>
                    <div
                      className={styles.needFill}
                      style={{ width: `${fill}%` }}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        <div className={styles.personaSection}>
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
        </div>
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
              <button
                key={ctrl.id}
                type="button"
                className={classNames(styles.chip, {
                  [styles.chipPrimary]: ctrl.style === "primary",
                })}
                onClick={() => onQuestControl(ctrl)}
              >
                {ctrl.label}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className={styles.actionRow}>
        <button type="button" className={styles.chip} onClick={onReroll}>
          Reroll personality
        </button>
        <button
          type="button"
          className={classNames(styles.chip, {
            [styles.chipPrimary]: settings?.proactive_enabled,
          })}
          onClick={onToggleProactive}
          disabled={isSavingSettings}
          aria-pressed={settings?.proactive_enabled}
        >
          Proactive {settings?.proactive_enabled ? "on" : "off"}
        </button>
        <button
          type="button"
          className={classNames(styles.chip, {
            [styles.chipPrimary]: !!settings?.personality_prompt,
          })}
          onClick={() =>
            onPromptChange(
              settings?.personality_prompt ? null : personality?.prompt ?? null,
            )
          }
          disabled={isSavingSettings}
          aria-pressed={!!settings?.personality_prompt}
        >
          Pin current vibe
        </button>
      </div>
    </div>
  </div>
);
