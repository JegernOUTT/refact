import React from "react";
import { Badge, Button, Surface, Text } from "../../components/ui";
import { SKILLS } from "./constants";
import type {
  BuddyControl,
  BuddyNeeds,
  BuddyPersonalityProfile,
  BuddyQuest,
  BuddySettings,
} from "./types";
import styles from "./BuddyPersonalityPanel.module.css";

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
  name: string;
  settings: BuddySettings | undefined;
  isSavingSettings: boolean;
  onQuestControl: (control: BuddyControl) => void;
  onReroll: () => void;
  onToggleProactive: () => void;
  onPromptChange: (prompt: string | null) => void;
}

const fillStyle = (fill: number): React.CSSProperties =>
  ({ "--buddy-fill": `${fill}%` }) as React.CSSProperties;

export const BuddyPersonalityPanel: React.FC<BuddyPersonalityPanelProps> = ({
  personality,
  needRows,
  unlockedSkills,
  activeQuest,
  name,
  settings,
  isSavingSettings,
  onQuestControl,
  onReroll,
  onToggleProactive,
  onPromptChange,
}) => (
  <div className={styles.outer} data-testid="buddy-personality-panel">
    <Surface
      className={styles.panel}
      animated="rise"
      radius="card"
      variant="glass"
    >
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
            {personality?.archetype_label ?? name}
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
                  <span className={styles.needName}>{item.label}</span>
                  <span className={styles.needValue}>{item.value}</span>
                </div>
                <div className={styles.needBar}>
                  <div
                    className={styles.needFill}
                    style={fillStyle(item.fill)}
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
                    <div className={styles.needFill} style={fillStyle(fill)} />
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
                <Badge key={id} tone="muted" className={styles.skillChip}>
                  {skill.icon} {skill.name}
                </Badge>
              ) : null;
            })}
          </div>
        </div>
      </div>

      {activeQuest && (
        <div className={styles.questCard}>
          <div className={styles.questHeader}>
            <div className={styles.panelTitleGroup}>
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
            <Badge tone="accent">+{activeQuest.reward_xp} growth</Badge>
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
              style={fillStyle(
                Math.min(
                  100,
                  (Math.max(0, activeQuest.progress) /
                    Math.max(1, activeQuest.goal)) *
                    100,
                ),
              )}
            />
          </div>

          <div className={styles.questControls}>
            {activeQuest.controls.map((ctrl) => (
              <Button
                key={ctrl.id}
                type="button"
                size="sm"
                variant={ctrl.style === "primary" ? "primary" : "ghost"}
                onClick={() => onQuestControl(ctrl)}
              >
                {ctrl.label}
              </Button>
            ))}
          </div>
        </div>
      )}

      <div className={styles.actionRow}>
        <Button type="button" size="sm" variant="ghost" onClick={onReroll}>
          Reroll personality
        </Button>
        <Button
          type="button"
          size="sm"
          variant={settings?.proactive_enabled ? "primary" : "ghost"}
          onClick={onToggleProactive}
          disabled={isSavingSettings}
          aria-pressed={settings?.proactive_enabled}
        >
          Proactive {settings?.proactive_enabled ? "on" : "off"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant={settings?.personality_prompt ? "primary" : "ghost"}
          onClick={() =>
            onPromptChange(
              settings?.personality_prompt ? null : personality?.prompt ?? null,
            )
          }
          disabled={isSavingSettings}
          aria-pressed={!!settings?.personality_prompt}
        >
          Pin current vibe
        </Button>
      </div>
    </Surface>
  </div>
);
