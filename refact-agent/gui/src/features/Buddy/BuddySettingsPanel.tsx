import React, { useCallback, useEffect, useRef, useState } from "react";
import { CalendarClock, Smile } from "lucide-react";
import {
  Button,
  FieldTextarea,
  SaveStatus,
  SegmentedControl,
  SettingItem,
  type SaveStatusState,
} from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  selectBuddySettings,
  selectBuddyState,
  selectBuddyStorage,
  selectChatReactionDebug,
} from "./buddySlice";
import { useUpdateBuddySettingsMutation } from "../../services/refact/buddy";
import type { QuietHoursMode } from "./types";
import { BuddySection, buddyName as resolveBuddyName } from "./ui";
import { CommitNumberField } from "./BuddySettingsFields";
import {
  BuddyChatGroup,
  BuddyGeneralGroup,
  BuddyHumorRows,
  BuddyObserversGroup,
  BuddyTopicsGroup,
  type BuddySettingsPatch,
} from "./BuddySettingsGroups";
import {
  BuddyJobTelemetry,
  BuddySpeechDecisions,
} from "./BuddySettingsActivity";
import {
  BuddyAdvancedDiagnostics,
  BuddyMutedRules,
} from "./BuddySettingsDiagnostics";
import styles from "./BuddySettingsPanel.module.css";

const PROMPT_DEBOUNCE_MS = 700;
const SAVED_RESET_MS = 2000;
const EMPTY_LIST: string[] = [];

const QUIET_HOURS_OPTIONS: { value: QuietHoursMode; label: string }[] = [
  { value: "off", label: "Off" },
  { value: "auto", label: "Auto" },
  { value: "fixed", label: "Fixed" },
];

const buildPromptPatch = (value: string): BuddySettingsPatch =>
  value.trim() === ""
    ? { clear_personality_prompt: true }
    : { personality_prompt: value };

function parseHour(raw: string): number | null | undefined {
  if (raw === "") return null;
  if (!/^\d{1,2}$/.test(raw)) return undefined;
  const n = Number(raw);
  return n >= 0 && n <= 23 ? n : undefined;
}

function parseBudget(raw: string): number | null | undefined {
  if (raw === "") return null;
  if (!/^\d{1,9}$/.test(raw)) return undefined;
  const n = Number(raw);
  return n > 0 ? n : null;
}

interface Props {
  onClose?: () => void;
}

export const BuddySettingsPanel: React.FC<Props> = ({ onClose }) => {
  const liveSettings = useAppSelector(selectBuddySettings);
  const storage = useAppSelector(selectBuddyStorage);
  const buddyState = useAppSelector(selectBuddyState);
  const chatReactionDebug = useAppSelector(selectChatReactionDebug);
  const name = resolveBuddyName(buddyState?.identity.name);
  const mutedRules = buddyState?.muted_rules ?? EMPTY_LIST;
  const summaries = buddyState?.workflow_summaries ?? [];
  const [updateSettingsMutation] = useUpdateBuddySettingsMutation();
  const [saveStatus, setSaveStatus] = useState<SaveStatusState>("idle");
  const [promptDraft, setPromptDraft] = useState<string>("");
  const [promptFocused, setPromptFocused] = useState(false);
  const [promptDirty, setPromptDirty] = useState(false);
  const promptDraftRef = useRef("");
  const promptBaselineRef = useRef("");
  const saveSeqRef = useRef(0);
  const promptDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const savedTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (promptFocused || promptDirty) return;
    const nextPrompt = liveSettings?.personality_prompt ?? "";
    promptDraftRef.current = nextPrompt;
    promptBaselineRef.current = nextPrompt;
    setPromptDraft(nextPrompt);
  }, [liveSettings?.personality_prompt, promptDirty, promptFocused]);

  useEffect(() => {
    return () => {
      if (promptDebounceRef.current !== null)
        clearTimeout(promptDebounceRef.current);
      if (savedTimerRef.current !== null) clearTimeout(savedTimerRef.current);
    };
  }, []);

  const autoSave = useCallback(
    async (patch: BuddySettingsPatch) => {
      const requestSeq = saveSeqRef.current + 1;
      saveSeqRef.current = requestSeq;
      setSaveStatus("saving");
      if (savedTimerRef.current !== null) clearTimeout(savedTimerRef.current);
      try {
        await updateSettingsMutation(patch).unwrap();
        if (saveSeqRef.current === requestSeq) {
          setSaveStatus("saved");
          savedTimerRef.current = setTimeout(() => {
            if (saveSeqRef.current === requestSeq) setSaveStatus("idle");
          }, SAVED_RESET_MS);
        }
        return true;
      } catch {
        if (saveSeqRef.current === requestSeq) setSaveStatus("error");
        return false;
      }
    },
    [updateSettingsMutation],
  );

  const savePromptValue = useCallback(
    async (value: string) => {
      if (value === promptBaselineRef.current) {
        setPromptDirty(false);
        return true;
      }
      const saved = await autoSave(buildPromptPatch(value));
      if (saved && promptDraftRef.current === value) {
        promptBaselineRef.current = value;
        setPromptDirty(false);
      }
      return saved;
    },
    [autoSave],
  );

  if (!liveSettings) return null;

  const patch = (next: BuddySettingsPatch) => {
    void autoSave(next);
  };

  const handlePromptChange = (val: string) => {
    setPromptDraft(val);
    promptDraftRef.current = val;
    setPromptDirty(true);
    if (promptDebounceRef.current !== null)
      clearTimeout(promptDebounceRef.current);
    promptDebounceRef.current = setTimeout(() => {
      promptDebounceRef.current = null;
      void savePromptValue(val);
    }, PROMPT_DEBOUNCE_MS);
  };

  const flushPrompt = (value: string) => {
    if (promptDebounceRef.current !== null) {
      clearTimeout(promptDebounceRef.current);
      promptDebounceRef.current = null;
    }
    void savePromptValue(value);
  };

  const handlePromptBlur = () => {
    setPromptFocused(false);
    flushPrompt(promptDraftRef.current);
  };

  const handlePromptClear = () => {
    setPromptDraft("");
    promptDraftRef.current = "";
    setPromptDirty(true);
    flushPrompt("");
  };

  const commitHour =
    (key: "daily_digest_hour" | "quiet_hours_start" | "quiet_hours_end") =>
    (raw: string) => {
      const value = parseHour(raw);
      if (value === undefined) return false;
      if (value === null && key !== "daily_digest_hour") return false;
      patch({ [key]: value });
      return true;
    };

  const commitBudget = (raw: string) => {
    const value = parseBudget(raw);
    if (value === undefined) return false;
    patch({ daily_llm_token_budget: value });
    return true;
  };

  const quietMode = liveSettings.quiet_hours_mode ?? "auto";
  const groupProps = { settings: liveSettings, name, onPatch: patch };

  return (
    <div className={styles.panel} data-testid="buddy-settings-panel">
      <div className={styles.headerRow}>
        <p className={styles.headerDescription}>
          Tune {name}&apos;s autonomy, observations, and tiny-chaos operating
          hours.
        </p>
        <span role={saveStatus === "error" ? "alert" : undefined}>
          <SaveStatus
            state={saveStatus}
            label={saveStatus === "error" ? "Save failed" : undefined}
          />
        </span>
      </div>

      <BuddyGeneralGroup {...groupProps} />
      <BuddyChatGroup {...groupProps} />

      <BuddySection title="Personality" icon={Smile}>
        <div className={styles.rows}>
          <BuddyHumorRows {...groupProps} />
          <SettingItem
            layout="stack"
            title="Personality prompt"
            description="Custom instructions save as you type."
            control={
              <FieldTextarea
                rows={3}
                placeholder="Custom personality instructions…"
                value={promptDraft}
                onChange={handlePromptChange}
                onFocus={() => setPromptFocused(true)}
                onBlur={handlePromptBlur}
                aria-label="Personality prompt"
                data-testid="buddy-personality-prompt"
              />
            }
          />
          {promptDraft ? (
            <div className={styles.promptActions}>
              <Button
                size="sm"
                variant="ghost"
                onClick={handlePromptClear}
                data-testid="buddy-clear-prompt"
              >
                Clear prompt
              </Button>
            </div>
          ) : null}
        </div>
      </BuddySection>

      <BuddySection title="Schedule and budget" icon={CalendarClock}>
        <div className={styles.rows} data-testid="buddy-quiet-hours">
          <SettingItem
            className={styles.inlineRow}
            title="Daily digest hour"
            description="Hour of the day, 0 to 23. Leave empty to turn it off."
            control={
              <CommitNumberField
                value={liveSettings.daily_digest_hour ?? null}
                onCommit={commitHour("daily_digest_hour")}
                ariaLabel="Daily digest hour"
                placeholder="Off"
                min={0}
                max={23}
                testId="buddy-digest-hour"
              />
            }
          />
          <SettingItem
            className={styles.controlRow}
            title="Quiet hours"
            description="Auto follows your detected activity pattern."
            control={
              <SegmentedControl
                aria-label="Quiet hours"
                name="buddy-quiet-hours-mode"
                options={QUIET_HOURS_OPTIONS}
                size="sm"
                value={quietMode}
                onValueChange={(value) =>
                  patch({ quiet_hours_mode: value as QuietHoursMode })
                }
              />
            }
          />
          {quietMode === "fixed" ? (
            <SettingItem
              className={styles.inlineRow}
              title="Quiet from and until"
              description="Hours of the day, 0 to 23."
              control={
                <span className={styles.hourPair}>
                  <CommitNumberField
                    value={liveSettings.quiet_hours_start ?? 22}
                    onCommit={commitHour("quiet_hours_start")}
                    ariaLabel="Quiet hours start"
                    min={0}
                    max={23}
                    testId="buddy-quiet-start"
                  />
                  to
                  <CommitNumberField
                    value={liveSettings.quiet_hours_end ?? 8}
                    onCommit={commitHour("quiet_hours_end")}
                    ariaLabel="Quiet hours end"
                    min={0}
                    max={23}
                    testId="buddy-quiet-end"
                  />
                </span>
              }
            />
          ) : null}
          <SettingItem
            className={styles.inlineRow}
            title="Daily model token budget"
            description="Background jobs pause after this many tokens a day. Leave empty for no limit."
            control={
              <CommitNumberField
                value={liveSettings.daily_llm_token_budget ?? null}
                onCommit={commitBudget}
                ariaLabel="Daily model token budget"
                placeholder="No limit"
                min={0}
                testId="buddy-token-budget"
              />
            }
          />
        </div>
      </BuddySection>

      <BuddyTopicsGroup {...groupProps} />
      <BuddyObserversGroup {...groupProps} />
      <BuddySpeechDecisions name={name} />
      <BuddyMutedRules rules={mutedRules} name={name} />
      <BuddyJobTelemetry summaries={summaries} />
      <BuddyAdvancedDiagnostics
        storage={storage ?? undefined}
        chatReactionDebug={chatReactionDebug}
        name={name}
      />

      {onClose ? (
        <div className={styles.footer}>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
      ) : null}
    </div>
  );
};
