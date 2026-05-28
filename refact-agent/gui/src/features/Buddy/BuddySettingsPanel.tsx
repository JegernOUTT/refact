import React, { useEffect, useState } from "react";
import { Text, Button, Switch } from "@radix-ui/themes";
import classNames from "classnames";
import { useAppSelector } from "../../hooks";
import { selectBuddySettings } from "./buddySlice";
import { useUpdateBuddySettingsMutation } from "../../services/refact/buddy";
import type { BuddySettings, HumorLevel, AutonomyLevel } from "./types";
import styles from "./BuddySettingsPanel.module.css";

const OBSERVER_LABELS: Record<keyof BuddySettings["observers"], string> = {
  task_health: "Task Health",
  trajectory_clutter: "Trajectory Clutter",
  chat_pattern: "Chat Pattern",
  customization_drift: "Customization Drift",
  memory_garden: "Memory Garden",
  mcp_auth: "MCP Auth",
  git_pressure: "Git Pressure",
  diagnostic_cluster: "Diagnostics",
  provider_health: "Provider Health",
};

interface Props {
  onClose?: () => void;
}

export const BuddySettingsPanel: React.FC<Props> = ({ onClose }) => {
  const liveSettings = useAppSelector(selectBuddySettings);
  const [updateSettings, { isLoading }] = useUpdateBuddySettingsMutation();
  const [cached, setCached] = useState<BuddySettings | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (liveSettings && cached === null) {
      setCached(liveSettings);
    }
  }, [liveSettings, cached]);

  const settings = cached ?? liveSettings;
  if (!settings) return null;

  const patch = <K extends keyof BuddySettings>(
    key: K,
    val: BuddySettings[K],
  ) => {
    setCached((prev) => (prev ? { ...prev, [key]: val } : prev));
  };

  const patchObserver = (
    key: keyof BuddySettings["observers"],
    val: boolean,
  ) => {
    setCached((prev) =>
      prev ? { ...prev, observers: { ...prev.observers, [key]: val } } : prev,
    );
  };

  const handleSave = async () => {
    if (!cached) return;
    setSaveError(null);
    try {
      await updateSettings(cached).unwrap();
      if (onClose) onClose();
    } catch {
      setSaveError("Failed to save Buddy settings. Please try again.");
    }
  };

  const handleCancel = () => {
    setCached(liveSettings);
    if (onClose) onClose();
  };

  return (
    <div className={styles.panel} data-testid="buddy-settings-panel">
      <Text size="1" weight="bold" color="gray" className={styles.label}>
        SETTINGS
      </Text>

      <div className={styles.section}>
        <div className={styles.row}>
          <Text size="2">Humor</Text>
          <Switch
            checked={settings.humor_enabled}
            onCheckedChange={(v) => patch("humor_enabled", v)}
            aria-label="humor enabled"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Humor level</Text>
          <div className={styles.radioGroup}>
            {(["off", "light", "normal"] as HumorLevel[]).map((lvl) => (
              <button
                key={lvl}
                type="button"
                className={classNames(styles.radioBtn, {
                  [styles.radioBtnActive]: settings.humor_level === lvl,
                })}
                onClick={() => patch("humor_level", lvl)}
              >
                {lvl}
              </button>
            ))}
          </div>
        </div>
        <div className={styles.row}>
          <Text size="2">Autonomy</Text>
          <div className={styles.radioGroup}>
            {(["read_only", "suggest", "safe_auto"] as AutonomyLevel[]).map(
              (lvl) => (
                <button
                  key={lvl}
                  type="button"
                  className={classNames(styles.radioBtn, {
                    [styles.radioBtnActive]: settings.autonomy_level === lvl,
                  })}
                  onClick={() => patch("autonomy_level", lvl)}
                >
                  {lvl}
                </button>
              ),
            )}
          </div>
        </div>
        <div className={styles.row}>
          <Text size="2">Quiet mode</Text>
          <Switch
            checked={settings.quiet_mode}
            onCheckedChange={(v) => patch("quiet_mode", v)}
            aria-label="quiet mode"
          />
        </div>
        <div className={styles.row}>
          <span className={styles.settingText}>
            <Text size="2">Chat pattern observation</Text>
            <small className={styles.settingDescription}>
              Periodic background scan for retry/stuck chat patterns.
              Independent from live chat reactions.
            </small>
          </span>
          <Switch
            checked={settings.message_observation_enabled}
            onCheckedChange={(v) => patch("message_observation_enabled", v)}
            aria-label="chat pattern observation enabled"
            data-testid="buddy-toggle-chat-pattern-observation"
          />
        </div>
        <div className={styles.row}>
          <span className={styles.settingText}>
            <Text size="2">Live chat reactions</Text>
            <small className={styles.settingDescription}>
              Pixel reacts to your messages with short comments, insights, or
              bug-candidate flags. Uses redacted input transiently and does not
              store it.
            </small>
          </span>
          <Switch
            checked={settings.chat_reactions_enabled}
            onCheckedChange={(v) => patch("chat_reactions_enabled", v)}
            aria-label="live chat reactions enabled"
            data-testid="buddy-toggle-live-chat-reactions"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Housekeeping</Text>
          <Switch
            checked={settings.housekeeping_enabled}
            onCheckedChange={(v) => patch("housekeeping_enabled", v)}
            aria-label="housekeeping enabled"
          />
        </div>
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          OBSERVERS
        </Text>
        <div className={styles.observersGrid}>
          {(
            Object.keys(OBSERVER_LABELS) as (keyof BuddySettings["observers"])[]
          ).map((key) => (
            <label key={key} className={styles.toggleRow}>
              <input
                type="checkbox"
                checked={settings.observers[key]}
                onChange={(e) => patchObserver(key, e.target.checked)}
                aria-label={OBSERVER_LABELS[key]}
              />
              <Text size="1">{OBSERVER_LABELS[key]}</Text>
            </label>
          ))}
        </div>
      </div>

      {saveError ? (
        <Text size="1" color="red" role="alert">
          {saveError}
        </Text>
      ) : null}

      <div className={styles.footer}>
        <Button size="1" variant="ghost" onClick={handleCancel}>
          Cancel
        </Button>
        <Button
          size="1"
          variant="soft"
          onClick={() => void handleSave()}
          disabled={isLoading}
        >
          Save
        </Button>
      </div>
    </div>
  );
};
