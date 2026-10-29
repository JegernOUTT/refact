import React from "react";
import { RefreshCw, Smile, Target } from "lucide-react";
import { Badge, Button, Icon, Text } from "../../components/ui";
import { SKILLS } from "./constants";
import { skillIcon } from "./buddyIcons";
import {
  BuddyEmptyState,
  BuddySection,
  BuddyStatusBadge,
  BuddySurface,
  buddyName,
  buddyTraitLabel,
  type BuddyStatusTone,
} from "./ui";
import type { NeedRow } from "./sections/buddyNeedRows";
import type {
  BuddyControl,
  BuddyPersonalityProfile,
  BuddyQuest,
  BuddySettings,
} from "./types";
import styles from "./BuddyPersonalityPanel.module.css";

interface BuddyPersonalityPanelProps {
  personality: BuddyPersonalityProfile | undefined;
  needRows: NeedRow[];
  unlockedSkills: string[];
  activeQuest: BuddyQuest | null;
  name: string;
  settings: BuddySettings | undefined;
  onQuestControl: (control: BuddyControl) => void;
  onReroll: () => void;
  onOpenSettings?: () => void;
}

function percent(value: number, max = 100): number {
  if (!Number.isFinite(value) || max <= 0) return 0;
  return Math.max(0, Math.min(100, Math.round((value / max) * 100)));
}

const Meter: React.FC<{
  label: string;
  value: React.ReactNode;
  fill: number;
  tone?: BuddyStatusTone;
}> = ({ label, value, fill, tone }) => (
  <div className={styles.meterRow}>
    <div className={styles.meterHeader}>
      <span className={styles.meterName}>{label}</span>
      <span className={styles.meterValue}>{value}</span>
    </div>
    <progress
      className={styles.meterBar}
      data-tone={tone ?? "accent"}
      value={fill}
      max={100}
      aria-label={label}
    />
  </div>
);

export const BuddyPersonalityPanel: React.FC<BuddyPersonalityPanelProps> = ({
  personality,
  needRows,
  unlockedSkills,
  activeQuest,
  name,
  settings,
  onQuestControl,
  onReroll,
  onOpenSettings,
}) => {
  const displayName = buddyName(name);
  const skills = unlockedSkills
    .map((id) => SKILLS.find((skill) => skill.id === id))
    .filter((skill): skill is (typeof SKILLS)[number] => Boolean(skill));
  return (
    <BuddySection
      title="Personality"
      icon={Smile}
      testId="buddy-personality-panel"
      className={styles.panel}
      action={
        <Button
          type="button"
          size="sm"
          variant="ghost"
          leftIcon={RefreshCw}
          onClick={onReroll}
        >
          Reroll personality
        </Button>
      }
    >
      <div className={styles.archetype}>
        <Text
          as="h3"
          size="2"
          weight="bold"
          className={styles.archetypeName}
          title={personality?.archetype_label ?? displayName}
        >
          {personality?.archetype_label ?? displayName}
        </Text>
        {personality?.vibe ? (
          <Text size="1" color="gray" className={styles.vibe}>
            {personality.vibe}
          </Text>
        ) : null}
        {personality?.summary ? (
          <Text size="1" className={styles.summary}>
            {personality.summary}
          </Text>
        ) : null}
      </div>

      <div className={styles.meterColumns}>
        <section className={styles.meterSection}>
          <Text as="h3" size="2" weight="bold" className={styles.sectionLabel}>
            Needs
          </Text>
          {needRows.map((item) => (
            <Meter
              key={item.key}
              label={item.label}
              value={item.value}
              fill={item.fill}
              tone={item.tone}
            />
          ))}
        </section>
        <section className={styles.meterSection}>
          <Text as="h3" size="2" weight="bold" className={styles.sectionLabel}>
            Traits
          </Text>
          {Object.entries(personality?.traits ?? {}).map(([key, value]) => {
            const score = Number(value);
            return (
              <Meter
                key={key}
                label={buddyTraitLabel(key)}
                value={Number.isFinite(score) ? score : 0}
                fill={percent(score)}
              />
            );
          })}
        </section>
      </div>

      <section className={styles.skillsSection}>
        <Text as="h3" size="2" weight="bold" className={styles.sectionLabel}>
          Skills
        </Text>
        {skills.length === 0 ? (
          <BuddyEmptyState compact title="No skills unlocked yet." />
        ) : (
          <div className={styles.skillsRow}>
            {skills.map((skill) => (
              <Badge
                key={skill.id}
                size="xs"
                tone="muted"
                className={styles.skillChip}
              >
                <Icon icon={skillIcon(skill.id)} size="sm" />
                {skill.name}
              </Badge>
            ))}
          </div>
        )}
      </section>

      {activeQuest ? (
        <BuddySurface level="inset" className={styles.questCard}>
          <div className={styles.questHeader}>
            <span className={styles.questTitleGroup}>
              <Icon icon={Target} size="sm" tone="muted" />
              <Text
                as="h3"
                size="2"
                weight="bold"
                className={styles.questTitle}
                title={activeQuest.title}
              >
                {activeQuest.title}
              </Text>
            </span>
            <Badge size="xs" tone="default">
              +{activeQuest.reward_xp} growth
            </Badge>
          </div>
          <Text size="1" className={styles.questDescription}>
            {activeQuest.description}
          </Text>
          <Meter
            label="Progress"
            value={`${Math.min(activeQuest.progress, activeQuest.goal)} / ${
              activeQuest.goal
            }`}
            fill={percent(
              Math.max(0, activeQuest.progress),
              Math.max(1, activeQuest.goal),
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
        </BuddySurface>
      ) : null}

      <div className={styles.stateRow} data-testid="buddy-personality-state">
        <span className={styles.stateItem}>
          Proactive suggestions
          <BuddyStatusBadge
            dimension="enabled"
            value={settings?.proactive_enabled ? "enabled" : "disabled"}
          />
        </span>
        <span className={styles.stateItem}>
          Custom personality prompt
          <BuddyStatusBadge
            dimension="enabled"
            value={settings?.personality_prompt ? "enabled" : "disabled"}
          />
        </span>
        {onOpenSettings ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={onOpenSettings}
            className={styles.stateLink}
          >
            Change in settings
          </Button>
        ) : null}
      </div>
    </BuddySection>
  );
};
