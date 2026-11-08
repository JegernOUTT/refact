import React from "react";
import { MessageSquare, Power, Radar, Sparkles } from "lucide-react";
import { SegmentedControl, SettingItem } from "../../components/ui";
import { BuddyScrollRail, BuddySection } from "./ui";
import { SwitchRow } from "./BuddySettingsFields";
import {
  OBSERVER_LABELS,
  SPEECH_INTENT_KEYS,
  humorIntentBlocked,
  isIntentAllowed,
  nextMutedIntents,
  speechIntentLabel,
} from "./BuddySettingsPanelLabels";
import type { AutonomyLevel, BuddySettings, HumorLevel } from "./types";
import styles from "./BuddySettingsPanel.module.css";

export type BuddySettingsPatch = Partial<BuddySettings> & {
  clear_personality_prompt?: boolean;
};

interface BuddySettingsGroupProps {
  settings: BuddySettings;
  name: string;
  onPatch: (patch: BuddySettingsPatch) => void;
}

const HUMOR_OPTIONS: { value: HumorLevel; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "light", label: "Light" },
  { value: "normal", label: "Normal" },
];

const AUTONOMY_OPTIONS: { value: AutonomyLevel; label: string }[] = [
  { value: "read_only", label: "Observe" },
  { value: "suggest", label: "Suggest" },
  { value: "propose", label: "Propose" },
  { value: "safe_auto", label: "Safe auto" },
];

export const BuddyGeneralGroup: React.FC<BuddySettingsGroupProps> = ({
  settings,
  name,
  onPatch,
}) => (
  <BuddySection title="General" icon={Power}>
    <div className={styles.rows}>
      <SwitchRow
        title={`${name} enabled`}
        checked={settings.enabled}
        onChange={(v) => onPatch({ enabled: v })}
        testId="buddy-toggle-enabled"
      />
      <SwitchRow
        title="Quiet mode"
        description={`${name} stays silent unless you ask.`}
        checked={settings.quiet_mode}
        onChange={(v) => onPatch({ quiet_mode: v })}
      />
      <SettingItem
        className={styles.controlRow}
        title="Autonomy"
        description={`How far ${name} may go on its own.`}
        control={
          <BuddyScrollRail watch={settings.autonomy_level}>
            <SegmentedControl
              aria-label="Autonomy"
              layout="content"
              name="buddy-autonomy-level"
              options={AUTONOMY_OPTIONS}
              size="sm"
              value={settings.autonomy_level}
              onValueChange={(value) =>
                onPatch({ autonomy_level: value as AutonomyLevel })
              }
            />
          </BuddyScrollRail>
        }
      />
      <SwitchRow
        title="Auto diagnostics"
        description="Look into errors as they happen."
        checked={settings.auto_diagnostics}
        onChange={(v) => onPatch({ auto_diagnostics: v })}
        testId="buddy-toggle-auto-diagnostics"
      />
      <SwitchRow
        title="Auto issue creation"
        description="Draft issues for problems it finds."
        checked={settings.auto_issue_creation}
        onChange={(v) => onPatch({ auto_issue_creation: v })}
        testId="buddy-toggle-auto-issue-creation"
      />
    </div>
  </BuddySection>
);

export const BuddyChatGroup: React.FC<BuddySettingsGroupProps> = ({
  settings,
  name,
  onPatch,
}) => (
  <BuddySection title="Chat and notifications" icon={MessageSquare}>
    <div className={styles.rows}>
      <SwitchRow
        title="Proactive suggestions"
        description={`${name} speaks up with ideas without being asked.`}
        checked={settings.proactive_enabled}
        onChange={(v) => onPatch({ proactive_enabled: v })}
        testId="buddy-toggle-proactive"
      />
      <SwitchRow
        title="Chat pattern observation"
        description="Periodic background scan for retry and stuck chat patterns. Independent from live chat reactions."
        checked={settings.message_observation_enabled}
        onChange={(v) => onPatch({ message_observation_enabled: v })}
        testId="buddy-toggle-chat-pattern-observation"
      />
      <SwitchRow
        title="Live chat reactions"
        description={`${name} reacts to your messages with short comments, insights, or bug-candidate flags. Uses redacted input transiently and does not store it.`}
        checked={settings.chat_reactions_enabled}
        onChange={(v) => onPatch({ chat_reactions_enabled: v })}
        testId="buddy-toggle-live-chat-reactions"
      />
      <SwitchRow
        title="Background chats"
        description={`Let ${name} start its own chats for background work.`}
        checked={settings.autonomous_chats_enabled}
        onChange={(v) => onPatch({ autonomous_chats_enabled: v })}
        testId="buddy-toggle-autonomous-chats"
      />
      <SwitchRow
        title="Housekeeping"
        description="Tidy up old chats and stale memories."
        checked={settings.housekeeping_enabled}
        onChange={(v) => onPatch({ housekeeping_enabled: v })}
      />
      <SwitchRow
        title="Topics and tags"
        description="Periodically organize project chats and tasks by theme."
        checked={settings.organization_enabled ?? true}
        onChange={(v) => onPatch({ organization_enabled: v })}
      />
    </div>
  </BuddySection>
);

export const BuddyTopicsGroup: React.FC<BuddySettingsGroupProps> = ({
  settings,
  name,
  onPatch,
}) => {
  const muted = settings.muted_intents ?? [];
  const allowedCount = SPEECH_INTENT_KEYS.filter((key) =>
    isIntentAllowed(muted, key),
  ).length;
  const humorBlocked = humorIntentBlocked(settings);
  const setAllowed = (key: string, allowed: boolean) =>
    onPatch({ muted_intents: nextMutedIntents(muted, key, allowed) });
  return (
    <BuddySection
      title={`What ${name} may talk about`}
      icon={Sparkles}
      count={allowedCount}
      description="Turn a topic off to keep it quiet."
      testId="buddy-muted-intents"
    >
      <div className={styles.toggleGrid}>
        {SPEECH_INTENT_KEYS.map((key) => {
          const label = speechIntentLabel(key);
          return (
            <SwitchRow
              key={key}
              title={label}
              ariaLabel={`Allow ${label.toLowerCase()}`}
              description={
                key === "humor" && humorBlocked
                  ? "Humor is off above."
                  : undefined
              }
              checked={isIntentAllowed(muted, key)}
              onChange={(v) => setAllowed(key, v)}
            />
          );
        })}
      </div>
    </BuddySection>
  );
};

export const BuddyHumorRows: React.FC<BuddySettingsGroupProps> = ({
  settings,
  onPatch,
}) => (
  <>
    <SwitchRow
      title="Humor"
      checked={settings.humor_enabled}
      onChange={(v) => onPatch({ humor_enabled: v })}
    />
    <SettingItem
      className={styles.controlRow}
      title="Humor level"
      control={
        <SegmentedControl
          aria-label="Humor level"
          name="buddy-humor-level"
          options={HUMOR_OPTIONS.map((option) => ({
            ...option,
            disabled: !settings.humor_enabled,
          }))}
          size="sm"
          value={settings.humor_level}
          onValueChange={(value) =>
            onPatch({ humor_level: value as HumorLevel })
          }
        />
      }
    />
  </>
);

export const BuddyObserversGroup: React.FC<BuddySettingsGroupProps> = ({
  settings,
  name,
  onPatch,
}) => {
  const keys = Object.keys(
    OBSERVER_LABELS,
  ) as (keyof BuddySettings["observers"])[];
  return (
    <BuddySection
      title="Observers"
      icon={Radar}
      count={keys.filter((key) => settings.observers[key]).length}
      description={`What ${name} keeps an eye on in the background.`}
    >
      <div className={styles.toggleGrid}>
        {keys.map((key) => (
          <SwitchRow
            key={key}
            title={OBSERVER_LABELS[key]}
            description={
              key === "chat_pattern" && !settings.message_observation_enabled
                ? "Needs chat pattern observation."
                : undefined
            }
            checked={settings.observers[key]}
            onChange={(v) =>
              onPatch({ observers: { ...settings.observers, [key]: v } })
            }
          />
        ))}
      </div>
    </BuddySection>
  );
};
