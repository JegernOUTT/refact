import React, { FormEvent, useMemo, useState } from "react";
import {
  Button,
  Field,
  FieldError,
  FieldSwitch,
  FieldText,
  FieldTextarea,
  SegmentedControl,
} from "../../components/ui";
import {
  type CreateCronRequest,
  schedulerErrorMessage,
} from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type CronPreset = "hourly" | "daily" | "weekdays" | "five-min" | "custom";
type ScheduleKind = "cron" | "interval" | "once";

type CronCreateFormData = Omit<CreateCronRequest, "chat_id" | "mode">;

export type CronCreateFormProps = {
  onSubmit: (request: CronCreateFormData) => Promise<void>;
  isLoading?: boolean;
  error?: unknown;
  taskCount: number;
  maxTasks?: number;
};

const PRESETS: Record<Exclude<CronPreset, "custom">, string> = {
  hourly: "7 * * * *",
  daily: "3 9 * * *",
  weekdays: "3 9 * * 1-5",
  "five-min": "*/5 * * * *",
};

const PRESET_OPTIONS: { value: CronPreset; label: string }[] = [
  { value: "hourly", label: "Hourly" },
  { value: "daily", label: "Daily 9am" },
  { value: "weekdays", label: "Weekdays 9am" },
  { value: "five-min", label: "Every 5 min" },
  { value: "custom", label: "Custom" },
];

const SCHEDULE_OPTIONS = [
  { value: "cron", label: "Cron" },
  { value: "interval", label: "Interval" },
  { value: "once", label: "One-shot" },
];

const CRON_PATTERN = /^\S+\s+\S+\s+\S+\s+\S+\s+\S+$/;
const INTERVAL_PATTERN = /^\d+\s*[smhd]$/;

function validateCron(value: string): string | null {
  if (!value.trim()) return "Cron expression is required.";
  if (!CRON_PATTERN.test(value.trim())) {
    return "Use a standard 5-field cron expression.";
  }
  return null;
}

function validateInterval(value: string): string | null {
  if (!value.trim()) return "Interval is required.";
  if (!INTERVAL_PATTERN.test(value.trim())) {
    return "Use a duration like 30m, 2h, or 1d.";
  }
  return null;
}

function validateOneShot(value: string): string | null {
  if (!value.trim()) return "One-shot time is required.";
  return null;
}

export const CronCreateForm: React.FC<CronCreateFormProps> = ({
  onSubmit,
  isLoading = false,
  error,
  taskCount,
  maxTasks = 50,
}) => {
  const [scheduleKind, setScheduleKind] = useState<ScheduleKind>("cron");
  const [preset, setPreset] = useState<CronPreset>("hourly");
  const [cron, setCron] = useState(PRESETS.hourly);
  const [every, setEvery] = useState("30m");
  const [at, setAt] = useState("in 30m");
  const [tz, setTz] = useState("");
  const [prompt, setPrompt] = useState("");
  const [description, setDescription] = useState("");
  const [recurring, setRecurring] = useState(true);
  const [durable, setDurable] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const capExceeded = taskCount >= maxTasks;

  const backendError = useMemo(() => {
    if (!error) return null;
    return schedulerErrorMessage(error);
  }, [error]);

  const setSelectedPreset = (value: CronPreset) => {
    setPreset(value);
    if (value !== "custom") {
      setCron(PRESETS[value]);
    }
  };

  const handleCronChange = (value: string) => {
    setCron(value);
    setPreset("custom");
  };

  const handleScheduleKindChange = (value: string) => {
    setScheduleKind(value as ScheduleKind);
    setLocalError(null);
  };

  const validateSchedule = (): string | null => {
    if (scheduleKind === "interval") return validateInterval(every);
    if (scheduleKind === "once") return validateOneShot(at);
    return validateCron(cron);
  };

  const buildScheduleRequest = (): Pick<
    CreateCronRequest,
    "cron" | "every" | "at" | "tz" | "recurring"
  > => {
    if (scheduleKind === "interval") {
      return { every: every.trim(), recurring };
    }
    if (scheduleKind === "once") {
      return { at: at.trim(), recurring: false };
    }
    return {
      cron: cron.trim(),
      recurring,
      ...(tz.trim() ? { tz: tz.trim() } : {}),
    };
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (capExceeded) {
      setLocalError(
        "Scheduler limit reached. Delete a task before creating another.",
      );
      return;
    }
    const scheduleError = validateSchedule();
    if (scheduleError) {
      setLocalError(scheduleError);
      return;
    }
    if (!description.trim()) {
      setLocalError("Description is required.");
      return;
    }
    if (description.length > 80) {
      setLocalError("Description must be 80 characters or less.");
      return;
    }
    if (!prompt.trim()) {
      setLocalError("Prompt is required.");
      return;
    }

    setLocalError(null);
    await onSubmit({
      ...buildScheduleRequest(),
      prompt: prompt.trim(),
      durable,
      description: description.trim(),
    });
  };

  const submitForm = (event: FormEvent<HTMLFormElement>) => {
    void handleSubmit(event);
  };

  const errorMessage = localError ?? backendError;

  return (
    <form className={styles.form} onSubmit={submitForm}>
      <p className={styles.sectionHint}>
        Use cron expressions for calendar schedules, intervals for repeated
        delays, or one-shot times for a single future prompt.
      </p>

      <Field label="Schedule kind">
        <SegmentedControl
          aria-label="Schedule kind"
          name="scheduler-schedule-kind"
          options={SCHEDULE_OPTIONS}
          value={scheduleKind}
          onValueChange={handleScheduleKindChange}
        />
      </Field>

      {scheduleKind === "cron" ? (
        <>
          <div
            className={styles.presetGroup}
            role="group"
            aria-label="Cron presets"
          >
            {PRESET_OPTIONS.map((option) => (
              <button
                className={styles.presetPill}
                type="button"
                key={option.value}
                aria-pressed={preset === option.value}
                onClick={() => setSelectedPreset(option.value)}
              >
                {option.label}
              </button>
            ))}
          </div>

          <Field
            label="Cron expression"
            helper="minute hour day month weekday"
            required
          >
            <FieldText
              className={styles.monoField}
              value={cron}
              onChange={handleCronChange}
              aria-label="Cron expression"
              placeholder="7 * * * *"
            />
          </Field>

          <Field label="Timezone" helper="Optional IANA timezone, such as UTC.">
            <FieldText
              className={styles.fieldControl}
              value={tz}
              onChange={setTz}
              aria-label="Timezone"
              placeholder="UTC"
            />
          </Field>
        </>
      ) : null}

      {scheduleKind === "interval" ? (
        <Field label="Interval" helper="Use s, m, h, or d suffixes." required>
          <FieldText
            className={styles.monoField}
            value={every}
            onChange={setEvery}
            aria-label="Interval"
            placeholder="30m"
          />
        </Field>
      ) : null}

      {scheduleKind === "once" ? (
        <Field
          label="One-shot time"
          helper="Use a relative time like in 30m or an RFC3339 timestamp."
          required
        >
          <FieldText
            className={styles.monoField}
            value={at}
            onChange={setAt}
            aria-label="One-shot time"
            placeholder="in 30m"
          />
        </Field>
      ) : null}

      <Field label="Description" helper={`${description.length}/80`} required>
        <FieldText
          className={styles.fieldControl}
          value={description}
          maxLength={80}
          onChange={setDescription}
          aria-label="Description"
        />
      </Field>

      <Field label="Prompt" required>
        <FieldTextarea
          className={styles.fieldControl}
          value={prompt}
          onChange={setPrompt}
          aria-label="Prompt"
          rows={4}
        />
      </Field>

      <div className={styles.toggles}>
        {scheduleKind !== "once" ? (
          <Field label="Recurring" helper="Run on every matching schedule.">
            <FieldSwitch
              checked={recurring}
              onChange={setRecurring}
              aria-label="Recurring"
            />
          </Field>
        ) : null}
        <Field label="Durable" helper="Persist this schedule for the project.">
          <FieldSwitch
            checked={durable}
            onChange={setDurable}
            aria-label="Durable"
          />
        </Field>
      </div>

      {errorMessage ? <FieldError>{errorMessage}</FieldError> : null}

      <div className={styles.formActions}>
        <Button
          type="submit"
          variant="primary"
          loading={isLoading}
          disabled={capExceeded}
        >
          {isLoading ? "Creating…" : "Create"}
        </Button>
      </div>
    </form>
  );
};
