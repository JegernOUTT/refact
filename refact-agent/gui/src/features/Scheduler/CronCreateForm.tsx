import React, { FormEvent, useMemo, useState } from "react";
import {
  Button,
  Field,
  FieldError,
  FieldSwitch,
  FieldText,
  FieldTextarea,
} from "../../components/ui";
import {
  type CreateCronRequest,
  schedulerErrorMessage,
} from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type CronPreset = "hourly" | "daily" | "weekdays" | "five-min" | "custom";

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

const CRON_PATTERN = /^\S+\s+\S+\s+\S+\s+\S+\s+\S+$/;

function validateCron(value: string): string | null {
  if (!value.trim()) return "Cron expression is required.";
  if (!CRON_PATTERN.test(value.trim())) {
    return "Use a standard 5-field cron expression.";
  }
  return null;
}

export const CronCreateForm: React.FC<CronCreateFormProps> = ({
  onSubmit,
  isLoading = false,
  error,
  taskCount,
  maxTasks = 50,
}) => {
  const [preset, setPreset] = useState<CronPreset>("hourly");
  const [cron, setCron] = useState(PRESETS.hourly);
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

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (capExceeded) {
      setLocalError(
        "Scheduler limit reached. Delete a task before creating another.",
      );
      return;
    }
    const cronError = validateCron(cron);
    if (cronError) {
      setLocalError(cronError);
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
      cron: cron.trim(),
      prompt: prompt.trim(),
      recurring,
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
        Use standard 5-field cron syntax. Examples: hourly{" "}
        <code>7 * * * *</code>, weekdays <code>3 9 * * 1-5</code>.
      </p>

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
        <Field label="Recurring" helper="Run on every matching schedule.">
          <FieldSwitch
            checked={recurring}
            onChange={setRecurring}
            aria-label="Recurring"
          />
        </Field>
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
