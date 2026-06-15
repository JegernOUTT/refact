import React from "react";
import { Pin, RefreshCw, Smile, Target, Zap } from "lucide-react";
import { Badge, Button, Icon, Surface, Text } from "../../components/ui";
import { SKILLS } from "./constants";
import { BuddySectionHeader } from "./BuddySectionHeader";
import { skillIcon } from "./buddyIcons";
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

const Meter: React.FC<{
  label: string;
  value: React.ReactNode;
  fill: number;
}> = ({ label, value, fill }) => (
  <div className={styles.meterRow}>
    <div className={styles.meterHeader}>
      <span className={styles.meterName}>{label}</span>
      <span className={styles.meterValue}>{value}</span>
    </div>
    <div className={styles.meterBar}>
      <div className={styles.meterFill} style={fillStyle(fill)} />
    </div>
  </div>
);

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
  <Surface
    className={styles.panel}
    data-testid="buddy-personality-panel"
    animated="rise"
    radius="card"
    variant="glass"
  >
    <BuddySectionHeader icon={Smile} label="Personality" />
    <div className={styles.archetype}>
      <Text size="2" weight="bold" className={styles.archetypeName}>
        {personality?.archetype_label ?? name}
      </Text>
      <Text size="1" color="gray" className={styles.vibe}>
        {personality?.vibe ?? "Playful, quirky, helpful"}
      </Text>
    </div>

    <div className={styles.scrollBody}>
      {personality?.summary && (
        <Text size="1" className={styles.summary}>
          {personality.summary}
        </Text>
      )}

      <div className={styles.meterColumns}>
        <section className={styles.meterSection}>
          <span className={styles.sectionLabel}>Needs</span>
          {needRows.map((item) => (
            <Meter
              key={item.key}
              label={item.label}
              value={item.value}
              fill={item.fill}
            />
          ))}
        </section>
        <section className={styles.meterSection}>
          <span className={styles.sectionLabel}>Traits</span>
          {Object.entries(personality?.traits ?? {}).map(([key, value]) => {
            const fill = Math.max(0, Math.min(100, Number(value) || 0));
            return <Meter key={key} label={key} value={value} fill={fill} />;
          })}
        </section>
      </div>

      <section className={styles.skillsSection}>
        <span className={styles.sectionLabel}>Skills</span>
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
                <Icon icon={skillIcon(id)} size="sm" />
                {skill.name}
              </Badge>
            ) : null;
          })}
        </div>
      </section>

      {activeQuest && (
        <div className={styles.questCard}>
          <div className={styles.questHeader}>
            <span className={styles.questTitleGroup}>
              <Icon icon={Target} size="sm" tone="accent" />
              <Text size="2" weight="bold" className={styles.questTitle}>
                {activeQuest.title}
              </Text>
            </span>
            <Badge tone="accent">+{activeQuest.reward_xp} growth</Badge>
          </div>

          <Text size="1" className={styles.questDescription}>
            {activeQuest.description}
          </Text>

          <Meter
            label="Progress"
            value={`${Math.min(activeQuest.progress, activeQuest.goal)} / ${
              activeQuest.goal
            }`}
            fill={Math.min(
              100,
              (Math.max(0, activeQuest.progress) /
                Math.max(1, activeQuest.goal)) *
                100,
            )}
          />

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
        <Button
          type="button"
          size="sm"
          variant="ghost"
          leftIcon={RefreshCw}
          onClick={onReroll}
        >
          Reroll personality
        </Button>
        <Button
          type="button"
          size="sm"
          variant={settings?.proactive_enabled ? "primary" : "ghost"}
          leftIcon={Zap}
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
          leftIcon={Pin}
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
    </div>
  </Surface>
);
