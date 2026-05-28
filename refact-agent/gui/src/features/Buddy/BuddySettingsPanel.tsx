import React, { useCallback, useEffect, useRef, useState } from "react";
import { Button, Switch, Text, TextArea } from "@radix-ui/themes";
import classNames from "classnames";
import { useAppSelector } from "../../hooks";
import { selectBuddySettings } from "./buddySlice";
import { useUpdateBuddySettingsMutation } from "../../services/refact/buddy";
import type { AutonomyLevel, BuddySettings, HumorLevel } from "./types";
import styles from "./BuddySettingsPanel.module.css";

const PROMPT_DEBOUNCE_MS = 700;

type SaveStatus = "idle" | "saving" | "saved" | "failed";

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
  const [updateSettingsMutation] = useUpdateBuddySettingsMutation();
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle");
  const [promptDraft, setPromptDraft] = useState<string>("");
  const promptDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    setPromptDraft(liveSettings?.personality_prompt ?? "");
  }, [liveSettings?.personality_prompt]);

  useEffect(() => {
    return () => {
      if (promptDebounceRef.current !== null)
        clearTimeout(promptDebounceRef.current);
      if (savedTimerRef.current !== null) clearTimeout(savedTimerRef.current);
    };
  }, []);

  const autoSave = useCallback(
    async (
      patch: Partial<BuddySettings> & { clear_personality_prompt?: boolean },
    ) => {
      setSaveStatus("saving");
      if (savedTimerRef.current !== null) clearTimeout(savedTimerRef.current);
      try {
        await updateSettingsMutation(patch).unwrap();
        setSaveStatus("saved");
        savedTimerRef.current = setTimeout(() => setSaveStatus("idle"), 2000);
      } catch {
        setSaveStatus("failed");
      }
    },
    [updateSettingsMutation],
  );

  if (!liveSettings) return null;

  const handleSwitch = (key: keyof BuddySettings, val: boolean) => {
    void autoSave({ [key]: val });
  };

  const handleSegmented = <K extends keyof BuddySettings>(
    key: K,
    val: BuddySettings[K],
  ) => {
    void autoSave({ [key]: val });
  };

  const handleObserver = (
    key: keyof BuddySettings["observers"],
    val: boolean,
  ) => {
    const nextObservers = { ...liveSettings.observers, [key]: val };
    void autoSave({ observers: nextObservers });
  };

  const handlePromptChange = (val: string) => {
    setPromptDraft(val);
    if (promptDebounceRef.current !== null)
      clearTimeout(promptDebounceRef.current);
    promptDebounceRef.current = setTimeout(() => {
      void autoSave({ personality_prompt: val || null });
    }, PROMPT_DEBOUNCE_MS);
  };

  const handlePromptBlur = () => {
    if (promptDebounceRef.current !== null) {
      clearTimeout(promptDebounceRef.current);
      promptDebounceRef.current = null;
    }
    void autoSave({ personality_prompt: promptDraft || null });
  };

  const handlePromptClear = () => {
    setPromptDraft("");
    if (promptDebounceRef.current !== null) {
      clearTimeout(promptDebounceRef.current);
      promptDebounceRef.current = null;
    }
    void autoSave({ clear_personality_prompt: true });
  };

  const handleDigestHourChange = (raw: string) => {
    if (raw === "") {
      void autoSave({ daily_digest_hour: null });
      return;
    }
    const n = parseInt(raw, 10);
    if (!Number.isNaN(n) && n >= 0 && n <= 23) {
      void autoSave({ daily_digest_hour: n });
    }
  };

  const saveLabel =
    saveStatus === "saving"
      ? "Saving…"
      : saveStatus === "saved"
        ? "Saved"
        : saveStatus === "failed"
          ? "Failed"
          : null;

  return (
    <div className={styles.panel} data-testid="buddy-settings-panel">
      <div className={styles.panelHeader}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          SETTINGS
        </Text>
        {saveLabel ? (
          <Text
            size="1"
            color={saveStatus === "failed" ? "red" : "gray"}
            role={saveStatus === "failed" ? "alert" : undefined}
            className={styles.saveStatus}
          >
            {saveLabel}
          </Text>
        ) : null}
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          CORE
        </Text>
        <div className={styles.row}>
          <Text size="2">Buddy enabled</Text>
          <Switch
            checked={liveSettings.enabled}
            onCheckedChange={(v) => handleSwitch("enabled", v)}
            aria-label="buddy enabled"
            data-testid="buddy-toggle-enabled"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Quiet mode</Text>
          <Switch
            checked={liveSettings.quiet_mode}
            onCheckedChange={(v) => handleSwitch("quiet_mode", v)}
            aria-label="quiet mode"
          />
        </div>
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          DIAGNOSTICS &amp; ISSUES
        </Text>
        <div className={styles.row}>
          <Text size="2">Auto diagnostics</Text>
          <Switch
            checked={liveSettings.auto_diagnostics}
            onCheckedChange={(v) => handleSwitch("auto_diagnostics", v)}
            aria-label="auto diagnostics"
            data-testid="buddy-toggle-auto-diagnostics"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Auto issue creation</Text>
          <Switch
            checked={liveSettings.auto_issue_creation}
            onCheckedChange={(v) => handleSwitch("auto_issue_creation", v)}
            aria-label="auto issue creation"
            data-testid="buddy-toggle-auto-issue-creation"
          />
        </div>
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          CHAT &amp; NOTIFICATIONS
        </Text>
        <div className={styles.row}>
          <Text size="2">Proactive suggestions</Text>
          <Switch
            checked={liveSettings.proactive_enabled}
            onCheckedChange={(v) => handleSwitch("proactive_enabled", v)}
            aria-label="proactive suggestions"
            data-testid="buddy-toggle-proactive"
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
            checked={liveSettings.message_observation_enabled}
            onCheckedChange={(v) =>
              handleSwitch("message_observation_enabled", v)
            }
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
            checked={liveSettings.chat_reactions_enabled}
            onCheckedChange={(v) => handleSwitch("chat_reactions_enabled", v)}
            aria-label="live chat reactions enabled"
            data-testid="buddy-toggle-live-chat-reactions"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Autonomous Buddy chats</Text>
          <Switch
            checked={liveSettings.autonomous_chats_enabled}
            onCheckedChange={(v) => handleSwitch("autonomous_chats_enabled", v)}
            aria-label="autonomous buddy chats"
            data-testid="buddy-toggle-autonomous-chats"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Housekeeping</Text>
          <Switch
            checked={liveSettings.housekeeping_enabled}
            onCheckedChange={(v) => handleSwitch("housekeeping_enabled", v)}
            aria-label="housekeeping enabled"
          />
        </div>
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          PERSONALITY
        </Text>
        <div className={styles.row}>
          <Text size="2">Humor</Text>
          <Switch
            checked={liveSettings.humor_enabled}
            onCheckedChange={(v) => handleSwitch("humor_enabled", v)}
            aria-label="humor enabled"
          />
        </div>
        <div className={styles.row}>
          <Text size="2">Humor level</Text>
          <div className={styles.radioGroup} role="group" aria-label="humor level">
            {(["off", "light", "normal"] as HumorLevel[]).map((lvl) => (
              <button
                key={lvl}
                type="button"
                aria-pressed={liveSettings.humor_level === lvl}
                className={classNames(styles.radioBtn, {
                  [styles.radioBtnActive]: liveSettings.humor_level === lvl,
                })}
                onClick={() => handleSegmented("humor_level", lvl)}
              >
                {lvl}
              </button>
            ))}
          </div>
        </div>
        <div className={styles.row}>
          <Text size="2">Autonomy</Text>
          <div className={styles.radioGroup} role="group" aria-label="autonomy level">
            {(["read_only", "suggest", "safe_auto"] as AutonomyLevel[]).map(
              (lvl) => (
                <button
                  key={lvl}
                  type="button"
                  aria-pressed={liveSettings.autonomy_level === lvl}
                  className={classNames(styles.radioBtn, {
                    [styles.radioBtnActive]:
                      liveSettings.autonomy_level === lvl,
                  })}
                  onClick={() => handleSegmented("autonomy_level", lvl)}
                >
                  {lvl}
                </button>
              ),
            )}
          </div>
        </div>
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          PERSONALITY PROMPT
        </Text>
        <TextArea
          size="1"
          rows={3}
          placeholder="Custom personality instructions…"
          value={promptDraft}
          onChange={(e) => handlePromptChange(e.target.value)}
          onBlur={handlePromptBlur}
          aria-label="personality prompt"
          data-testid="buddy-personality-prompt"
        />
        {promptDraft ? (
          <Button
            size="1"
            variant="ghost"
            onClick={handlePromptClear}
            data-testid="buddy-clear-prompt"
          >
            Clear
          </Button>
        ) : null}
      </div>

      <div className={styles.section}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          SCHEDULE
        </Text>
        <div className={styles.row}>
          <Text size="2">Daily digest hour (0–23)</Text>
          <input
            type="number"
            min={0}
            max={23}
            className={styles.digestInput}
            value={liveSettings.daily_digest_hour ?? ""}
            onChange={(e) => handleDigestHourChange(e.target.value)}
            aria-label="daily digest hour"
            placeholder="off"
            data-testid="buddy-digest-hour"
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
                checked={liveSettings.observers[key]}
                onChange={(e) => handleObserver(key, e.target.checked)}
                aria-label={OBSERVER_LABELS[key]}
              />
              <Text size="1">{OBSERVER_LABELS[key]}</Text>
            </label>
          ))}
        </div>
      </div>

      {onClose ? (
        <div className={styles.footer}>
          <Button size="1" variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
      ) : null}
    </div>
  );
};
