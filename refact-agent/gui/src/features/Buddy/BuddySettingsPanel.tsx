import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Button,
  DataTable,
  Field,
  FieldTextarea,
  SegmentedControl,
  Surface,
  Switch,
  Text,
} from "../../components/ui";
import type { DataTableColumn } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  selectBuddySettings,
  selectBuddyState,
  selectBuddyStorage,
  selectChatReactionDebug,
} from "./buddySlice";
import {
  useGetSpeechDecisionsQuery,
  useUnmuteBuddyRuleMutation,
  useUpdateBuddySettingsMutation,
} from "../../services/refact/buddy";
import type {
  AutonomyLevel,
  BuddySettings,
  BuddyWorkflowSummary,
  HumorLevel,
  QuietHoursMode,
} from "./types";
import styles from "./BuddySettingsPanel.module.css";

function formatCount(value: number | undefined): string {
  const count = value ?? 0;
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${(count / 1_000).toFixed(1)}K`;
  return String(count);
}

function formatWhen(value: string | null | undefined): string {
  if (!value) return "—";
  return value.slice(0, 16).replace("T", " ");
}

function totalTokens(row: BuddyWorkflowSummary): number {
  return (row.tokens_in ?? 0) + (row.tokens_out ?? 0);
}

const TELEMETRY_COLUMNS: DataTableColumn<BuddyWorkflowSummary>[] = [
  {
    id: "job",
    header: "Job",
    cell: (row) => row.workflow_id,
    sortValue: (row) => row.workflow_id,
  },
  {
    id: "runs",
    header: "Runs",
    cell: (row) => formatCount(row.run_count),
    sortValue: (row) => row.run_count,
    align: "end",
  },
  {
    id: "llm_calls",
    header: "LLM calls",
    cell: (row) => formatCount(row.llm_calls),
    sortValue: (row) => row.llm_calls ?? 0,
    align: "end",
  },
  {
    id: "tokens_in",
    header: "Tokens in",
    cell: (row) => formatCount(row.tokens_in),
    sortValue: (row) => row.tokens_in ?? 0,
    align: "end",
  },
  {
    id: "tokens_out",
    header: "Tokens out",
    cell: (row) => formatCount(row.tokens_out),
    sortValue: (row) => row.tokens_out ?? 0,
    align: "end",
  },
  {
    id: "outputs",
    header: "Outputs",
    cell: (row) => formatCount(row.outputs),
    sortValue: (row) => row.outputs ?? 0,
    align: "end",
  },
  {
    id: "last_output",
    header: "Last output",
    cell: (row) => formatWhen(row.last_output_at),
    sortValue: (row) => row.last_output_at ?? "",
    align: "end",
  },
];

const PROMPT_DEBOUNCE_MS = 700;

type SaveStatus = "idle" | "saving" | "saved" | "failed";

type BuddySettingsPatch = Partial<BuddySettings> & {
  clear_personality_prompt?: boolean;
};

const HUMOR_OPTIONS: Array<{ value: HumorLevel; label: string }> = [
  { value: "off", label: "Off" },
  { value: "light", label: "Light" },
  { value: "normal", label: "Normal" },
];

const AUTONOMY_OPTIONS: Array<{ value: AutonomyLevel; label: string }> = [
  { value: "read_only", label: "Observe" },
  { value: "suggest", label: "Suggest" },
  { value: "propose", label: "Propose" },
  { value: "safe_auto", label: "Safe auto" },
];

const QUIET_HOURS_OPTIONS: Array<{ value: QuietHoursMode; label: string }> = [
  { value: "off", label: "Off" },
  { value: "auto", label: "Auto" },
  { value: "fixed", label: "Fixed" },
];

const SPEECH_INTENT_KEYS: string[] = [
  "humor",
  "suggestion",
  "insight",
  "win",
  "error_alert",
  "greeting",
  "tour",
  "milestone",
  "memory_pulse_commentary",
  "quest_accept",
  "quest_complete",
  "chat_reaction",
];

const SPEECH_INTENT_LABELS: Record<string, string> = {
  chat_reaction: "Chat reactions",
};

const buildPromptPatch = (value: string): BuddySettingsPatch => {
  if (value.trim() === "") return { clear_personality_prompt: true };
  return { personality_prompt: value };
};

function formatChatReactionStatus(
  debug: ReturnType<typeof selectChatReactionDebug>,
): string {
  if (!debug) return "No chat reaction diagnostics yet.";
  const emitted = debug.counts_by_result.emitted;
  const skipped = debug.counts_by_result.skipped;
  const last = debug.recent_attempts.at(-1);
  if (last?.result === "emitted") {
    return `Last emitted ${last.signal_type ?? "chat reaction"}.`;
  }
  if (debug.last_skip_reason) {
    return `Last skipped: ${debug.last_skip_reason}.`;
  }
  return `Emitted ${emitted}, skipped ${skipped}.`;
}

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
  const storage = useAppSelector(selectBuddyStorage);
  const buddyState = useAppSelector(selectBuddyState);
  const telemetryRows = useMemo(
    () =>
      [...(buddyState?.workflow_summaries ?? [])].sort(
        (a, b) => totalTokens(b) - totalTokens(a),
      ),
    [buddyState],
  );
  const chatReactionDebug = useAppSelector(selectChatReactionDebug);
  const mutedRules = useMemo(() => buddyState?.muted_rules ?? [], [buddyState]);
  const mutedIntents = useMemo(
    () => liveSettings?.muted_intents ?? [],
    [liveSettings],
  );
  const { data: speechDecisionsData } = useGetSpeechDecisionsQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const speechDecisions = useMemo(
    () => [...(speechDecisionsData ?? [])].reverse().slice(0, 20),
    [speechDecisionsData],
  );
  const [updateSettingsMutation] = useUpdateBuddySettingsMutation();
  const [unmuteRuleMutation] = useUnmuteBuddyRuleMutation();
  const handleUnmute = useCallback(
    async (ruleKey: string) => {
      try {
        await unmuteRuleMutation({ rule_key: ruleKey }).unwrap();
      } catch {
        return;
      }
    },
    [unmuteRuleMutation],
  );
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle");
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
          }, 2000);
        }
        return true;
      } catch {
        if (saveSeqRef.current === requestSeq) setSaveStatus("failed");
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
    promptDraftRef.current = val;
    setPromptDirty(true);
    if (promptDebounceRef.current !== null)
      clearTimeout(promptDebounceRef.current);
    promptDebounceRef.current = setTimeout(() => {
      promptDebounceRef.current = null;
      void savePromptValue(val);
    }, PROMPT_DEBOUNCE_MS);
  };

  const handlePromptBlur = () => {
    setPromptFocused(false);
    if (promptDebounceRef.current !== null) {
      clearTimeout(promptDebounceRef.current);
      promptDebounceRef.current = null;
    }
    void savePromptValue(promptDraftRef.current);
  };

  const handlePromptClear = () => {
    setPromptDraft("");
    promptDraftRef.current = "";
    setPromptDirty(true);
    if (promptDebounceRef.current !== null) {
      clearTimeout(promptDebounceRef.current);
      promptDebounceRef.current = null;
    }
    void savePromptValue("");
  };

  const handleDigestHourChange = (raw: string, badInput: boolean) => {
    if (raw === "") {
      if (!badInput) void autoSave({ daily_digest_hour: null });
      return;
    }
    if (!/^\d{1,2}$/.test(raw)) return;
    const n = Number(raw);
    if (n >= 0 && n <= 23) {
      void autoSave({ daily_digest_hour: n });
    }
  };

  const handleQuietHourChange = (
    key: "quiet_hours_start" | "quiet_hours_end",
    raw: string,
  ) => {
    if (!/^\d{1,2}$/.test(raw)) return;
    const n = Number(raw);
    if (n >= 0 && n <= 23) {
      void autoSave({ [key]: n });
    }
  };

  const handleBudgetChange = (raw: string) => {
    if (raw === "") {
      void autoSave({ daily_llm_token_budget: null });
      return;
    }
    if (!/^\d{1,9}$/.test(raw)) return;
    const n = Number(raw);
    void autoSave({ daily_llm_token_budget: n > 0 ? n : null });
  };

  const handleIntentMute = (key: string, muted: boolean) => {
    const next = muted
      ? [...mutedIntents.filter((k) => k !== key), key]
      : mutedIntents.filter((k) => k !== key);
    void autoSave({ muted_intents: next });
  };

  const saveLabel =
    saveStatus === "saving"
      ? "Saving…"
      : saveStatus === "saved"
        ? "Saved to active Buddy settings"
        : saveStatus === "failed"
          ? "Save failed"
          : null;

  return (
    <Surface
      className={styles.panel}
      data-testid="buddy-settings-panel"
      radius="card"
      variant="glass"
    >
      <div className={styles.panelHeader}>
        <div className={styles.headerCopy}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            SETTINGS
          </Text>
          <Text size="1" className={styles.headerDescription}>
            Tune Pixel's autonomy, observations, and tiny-chaos operating hours.
          </Text>
        </div>
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

      <div className={`${styles.sectionGrid} rf-stagger`}>
        <div className={`${styles.section} rf-enter-rise`}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            CORE
          </Text>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Buddy enabled</Text>
            </span>
            <Switch
              checked={liveSettings.enabled}
              onCheckedChange={(v) => handleSwitch("enabled", v)}
              aria-label="buddy enabled"
              data-testid="buddy-toggle-enabled"
            />
          </div>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Quiet mode</Text>
            </span>
            <Switch
              checked={liveSettings.quiet_mode}
              onCheckedChange={(v) => handleSwitch("quiet_mode", v)}
              aria-label="quiet mode"
            />
          </div>
        </div>

        <div className={`${styles.section} rf-enter-rise`}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            DIAGNOSTICS &amp; ISSUES
          </Text>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Auto diagnostics</Text>
            </span>
            <Switch
              checked={liveSettings.auto_diagnostics}
              onCheckedChange={(v) => handleSwitch("auto_diagnostics", v)}
              aria-label="auto diagnostics"
              data-testid="buddy-toggle-auto-diagnostics"
            />
          </div>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Auto issue creation</Text>
            </span>
            <Switch
              checked={liveSettings.auto_issue_creation}
              onCheckedChange={(v) => handleSwitch("auto_issue_creation", v)}
              aria-label="auto issue creation"
              data-testid="buddy-toggle-auto-issue-creation"
            />
          </div>
        </div>

        <div className={`${styles.sectionWide} rf-enter-rise`}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            CHAT &amp; NOTIFICATIONS
          </Text>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Proactive suggestions</Text>
            </span>
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
                bug-candidate flags. Uses redacted input transiently and does
                not store it.
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
            <span className={styles.settingText}>
              <Text size="2">Autonomous Buddy chats</Text>
            </span>
            <Switch
              checked={liveSettings.autonomous_chats_enabled}
              onCheckedChange={(v) =>
                handleSwitch("autonomous_chats_enabled", v)
              }
              aria-label="autonomous buddy chats"
              data-testid="buddy-toggle-autonomous-chats"
            />
          </div>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Housekeeping</Text>
            </span>
            <Switch
              checked={liveSettings.housekeeping_enabled}
              onCheckedChange={(v) => handleSwitch("housekeeping_enabled", v)}
              aria-label="housekeeping enabled"
            />
          </div>
        </div>

        <div className={`${styles.section} rf-enter-rise`}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            PERSONALITY
          </Text>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Humor</Text>
            </span>
            <Switch
              checked={liveSettings.humor_enabled}
              onCheckedChange={(v) => handleSwitch("humor_enabled", v)}
              aria-label="humor enabled"
            />
          </div>
          <div className={`${styles.row} ${styles.segmentedRow}`}>
            <span className={styles.settingText}>
              <Text size="2">Humor level</Text>
            </span>
            <SegmentedControl
              aria-label="humor level"
              className={styles.segmentedControl}
              name="buddy-humor-level"
              options={HUMOR_OPTIONS}
              size="sm"
              value={liveSettings.humor_level}
              onValueChange={(value) =>
                handleSegmented("humor_level", value as HumorLevel)
              }
            />
          </div>
          <div className={`${styles.row} ${styles.segmentedRow}`}>
            <span className={styles.settingText}>
              <Text size="2">Autonomy</Text>
            </span>
            <SegmentedControl
              aria-label="autonomy level"
              className={styles.segmentedControl}
              name="buddy-autonomy-level"
              options={AUTONOMY_OPTIONS}
              size="sm"
              value={liveSettings.autonomy_level}
              onValueChange={(value) =>
                handleSegmented("autonomy_level", value as AutonomyLevel)
              }
            />
          </div>
        </div>

        <div className={`${styles.section} rf-enter-rise`}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            SCHEDULE
          </Text>
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Daily digest hour</Text>
              <small className={styles.settingDescription}>
                0–23, blank disables
              </small>
            </span>
            <input
              type="number"
              min={0}
              max={23}
              className={styles.digestInput}
              value={liveSettings.daily_digest_hour ?? ""}
              onChange={(e) =>
                handleDigestHourChange(
                  e.target.value,
                  e.target.validity.badInput,
                )
              }
              aria-label="daily digest hour"
              placeholder="off"
              data-testid="buddy-digest-hour"
            />
          </div>
        </div>

        <div
          className={`${styles.section} rf-enter-rise`}
          data-testid="buddy-quiet-hours"
        >
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            QUIET HOURS &amp; BUDGET
          </Text>
          <div className={`${styles.row} ${styles.segmentedRow}`}>
            <span className={styles.settingText}>
              <Text size="2">Quiet hours</Text>
              <small className={styles.settingDescription}>
                Auto follows your detected activity pattern
              </small>
            </span>
            <SegmentedControl
              aria-label="quiet hours mode"
              className={styles.segmentedControl}
              name="buddy-quiet-hours-mode"
              options={QUIET_HOURS_OPTIONS}
              size="sm"
              value={liveSettings.quiet_hours_mode ?? "auto"}
              onValueChange={(value) =>
                handleSegmented("quiet_hours_mode", value as QuietHoursMode)
              }
            />
          </div>
          {(liveSettings.quiet_hours_mode ?? "auto") === "fixed" && (
            <div className={styles.row}>
              <span className={styles.settingText}>
                <Text size="2">Quiet from / until</Text>
              </span>
              <input
                type="number"
                min={0}
                max={23}
                className={styles.digestInput}
                value={liveSettings.quiet_hours_start ?? 22}
                onChange={(e) =>
                  handleQuietHourChange("quiet_hours_start", e.target.value)
                }
                aria-label="quiet hours start"
                data-testid="buddy-quiet-start"
              />
              <input
                type="number"
                min={0}
                max={23}
                className={styles.digestInput}
                value={liveSettings.quiet_hours_end ?? 8}
                onChange={(e) =>
                  handleQuietHourChange("quiet_hours_end", e.target.value)
                }
                aria-label="quiet hours end"
                data-testid="buddy-quiet-end"
              />
            </div>
          )}
          <div className={styles.row}>
            <span className={styles.settingText}>
              <Text size="2">Daily LLM token budget</Text>
              <small className={styles.settingDescription}>
                Autonomous jobs pause past this; blank = unlimited
              </small>
            </span>
            <input
              type="number"
              min={0}
              className={styles.digestInput}
              value={liveSettings.daily_llm_token_budget ?? ""}
              onChange={(e) => handleBudgetChange(e.target.value)}
              aria-label="daily llm token budget"
              placeholder="∞"
              data-testid="buddy-token-budget"
            />
          </div>
        </div>

        <div
          className={`${styles.sectionWide} rf-enter-rise`}
          data-testid="buddy-muted-intents"
        >
          <div className={styles.sectionHeader}>
            <Text size="1" weight="bold" color="gray" className={styles.label}>
              MUTED SPEECH INTENTS
            </Text>
            <Text size="1" color="gray">
              {mutedIntents.length} muted
            </Text>
          </div>
          <div className={styles.observersGrid}>
            {SPEECH_INTENT_KEYS.map((key) => (
              <div key={key} className={styles.toggleRow}>
                <Text size="1" className={styles.toggleLabel}>
                  {SPEECH_INTENT_LABELS[key] ?? key.replace(/_/g, " ")}
                </Text>
                <Switch
                  checked={mutedIntents.includes(key)}
                  onCheckedChange={(v) => handleIntentMute(key, v)}
                  aria-label={`Mute ${key}`}
                />
              </div>
            ))}
          </div>
        </div>

        <div
          className={`${styles.sectionWide} rf-enter-rise`}
          data-testid="buddy-speech-decisions"
        >
          <div className={styles.sectionHeader}>
            <Text size="1" weight="bold" color="gray" className={styles.label}>
              WHY DID BUDDY SAY THAT?
            </Text>
            <Text size="1" color="gray">
              last {speechDecisions.length}
            </Text>
          </div>
          {speechDecisions.length === 0 ? (
            <Text size="1" color="gray">
              No speech decisions recorded yet.
            </Text>
          ) : (
            <div className={styles.observersGrid}>
              {speechDecisions.map((decision, idx) => (
                <div key={`${decision.at}-${idx}`} className={styles.toggleRow}>
                  <Text size="1" className={styles.toggleLabel}>
                    {decision.allowed ? "✓" : "✕"} [{decision.reason}]
                    {decision.intent ? ` ${decision.intent}:` : ""}{" "}
                    {decision.preview}
                  </Text>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className={`${styles.sectionWide} rf-enter-rise`}>
          <Field
            label={
              <Text
                size="1"
                weight="bold"
                color="gray"
                className={styles.label}
              >
                PERSONALITY PROMPT
              </Text>
            }
            helper="Custom instructions are autosaved after edits and committed immediately on blur."
          >
            <FieldTextarea
              rows={3}
              placeholder="Custom personality instructions…"
              value={promptDraft}
              onChange={handlePromptChange}
              onFocus={() => setPromptFocused(true)}
              onBlur={handlePromptBlur}
              aria-label="personality prompt"
              data-testid="buddy-personality-prompt"
            />
          </Field>
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

        <div className={`${styles.sectionWide} rf-enter-rise`}>
          <div className={styles.sectionHeader}>
            <Text size="1" weight="bold" color="gray" className={styles.label}>
              OBSERVERS
            </Text>
            <Text size="1" color="gray">
              {Object.values(liveSettings.observers).filter(Boolean).length}{" "}
              active
            </Text>
          </div>
          <div className={styles.observersGrid}>
            {(
              Object.keys(
                OBSERVER_LABELS,
              ) as (keyof BuddySettings["observers"])[]
            ).map((key) => (
              <div key={key} className={styles.toggleRow}>
                <Text size="1" className={styles.toggleLabel}>
                  {OBSERVER_LABELS[key]}
                </Text>
                <Switch
                  checked={liveSettings.observers[key]}
                  onCheckedChange={(v) => handleObserver(key, v)}
                  aria-label={OBSERVER_LABELS[key]}
                />
              </div>
            ))}
          </div>
        </div>

        <div
          className={`${styles.sectionWide} rf-enter-rise`}
          data-testid="buddy-job-telemetry"
        >
          <div className={styles.sectionHeader}>
            <Text size="1" weight="bold" color="gray" className={styles.label}>
              JOB TELEMETRY
            </Text>
            <Text size="1" color="gray">
              {telemetryRows.length} jobs
            </Text>
          </div>
          <DataTable
            columns={TELEMETRY_COLUMNS}
            rows={telemetryRows}
            getRowId={(row) => row.workflow_id}
            enableSorting
            emptyMessage="No job runs recorded yet"
          />
        </div>

        <div
          className={`${styles.sectionWide} rf-enter-rise`}
          data-testid="buddy-muted-rules"
        >
          <div className={styles.sectionHeader}>
            <Text size="1" weight="bold" color="gray" className={styles.label}>
              MUTED RULES
            </Text>
            <Text size="1" color="gray">
              {mutedRules.length} muted
            </Text>
          </div>
          {mutedRules.length === 0 ? (
            <Text size="1" color="gray">
              No rules muted. Use “Never” on a card to mute its rule.
            </Text>
          ) : (
            <div className={styles.observersGrid}>
              {mutedRules.map((ruleKey) => (
                <div key={ruleKey} className={styles.toggleRow}>
                  <Text size="1" className={styles.toggleLabel}>
                    {ruleKey}
                  </Text>
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={`Unmute ${ruleKey}`}
                    onClick={() => void handleUnmute(ruleKey)}
                  >
                    Unmute
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>

        <div
          className={`${styles.sectionWide} rf-enter-rise`}
          data-testid="buddy-storage-diagnostics"
        >
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            ADVANCED / DIAGNOSTICS
          </Text>
          {storage ? (
            <div className={styles.diagnosticsGrid}>
              <Text size="1" color="gray">
                Active Buddy folder
              </Text>
              <code className={styles.pathValue}>{storage.buddy_dir}</code>
              <Text size="1" color="gray">
                Settings file
              </Text>
              <code className={styles.pathValue}>{storage.settings_path}</code>
              <Text size="1" color="gray">
                Project root
              </Text>
              <code className={styles.pathValue}>{storage.project_root}</code>
            </div>
          ) : (
            <Text size="1" color="gray">
              Storage metadata is unavailable from this engine response.
            </Text>
          )}
          <div
            className={styles.diagnosticsGrid}
            data-testid="buddy-chat-reaction-diagnostics"
          >
            <Text size="1" color="gray">
              Chat reactions
            </Text>
            <Text size="1">{formatChatReactionStatus(chatReactionDebug)}</Text>
            {chatReactionDebug?.last_emitted_at ? (
              <>
                <Text size="1" color="gray">
                  Last emitted
                </Text>
                <code className={styles.pathValue}>
                  {chatReactionDebug.last_emitted_at}
                </code>
              </>
            ) : null}
          </div>
        </div>
      </div>

      {onClose ? (
        <div className={styles.footer}>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
      ) : null}
    </Surface>
  );
};
