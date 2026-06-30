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
type TriggerKind = "cron" | "interval" | "once" | "webhook";
type ActionKind = "agent" | "command";
type DeliveryKind = "chat" | "webhook" | "notifier";

export type JobBuilderFormData = Omit<CreateCronRequest, "chat_id" | "mode">;

export type JobBuilderProps = {
  onSubmit: (request: JobBuilderFormData) => Promise<void>;
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

const TRIGGER_OPTIONS: { value: TriggerKind; label: string }[] = [
  { value: "cron", label: "Cron" },
  { value: "interval", label: "Interval" },
  { value: "once", label: "One-shot" },
  { value: "webhook", label: "Webhook" },
];

const ACTION_OPTIONS: {
  value: ActionKind;
  label: string;
  ariaLabel: string;
}[] = [
  { value: "agent", label: "Agent turn", ariaLabel: "Agent turn action" },
  { value: "command", label: "Command", ariaLabel: "Command action" },
];

const DELIVERY_OPTIONS: {
  value: DeliveryKind;
  label: string;
  ariaLabel: string;
}[] = [
  { value: "chat", label: "Chat", ariaLabel: "Chat delivery" },
  { value: "webhook", label: "Webhook", ariaLabel: "Webhook delivery" },
  { value: "notifier", label: "Notifier", ariaLabel: "Notifier delivery" },
];

const CRON_PATTERN = /^\S+\s+\S+\s+\S+\s+\S+\s+\S+$/;
const INTERVAL_PATTERN = /^\d+\s*[smhd]$/;
const POSITIVE_INTEGER_PATTERN = /^\d+$/;

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

function parseTimeout(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  return Number(trimmed);
}

function isTriggerKind(value: string): value is TriggerKind {
  return TRIGGER_OPTIONS.some((option) => option.value === value);
}

function isActionKind(value: string): value is ActionKind {
  return ACTION_OPTIONS.some((option) => option.value === value);
}

function isDeliveryKind(value: string): value is DeliveryKind {
  return DELIVERY_OPTIONS.some((option) => option.value === value);
}

export const JobBuilder: React.FC<JobBuilderProps> = ({
  onSubmit,
  isLoading = false,
  error,
  taskCount,
  maxTasks = 50,
}) => {
  const [triggerKind, setTriggerKind] = useState<TriggerKind>("cron");
  const [actionKind, setActionKind] = useState<ActionKind>("agent");
  const [preset, setPreset] = useState<CronPreset>("hourly");
  const [cron, setCron] = useState(PRESETS.hourly);
  const [every, setEvery] = useState("30m");
  const [at, setAt] = useState("in 30m");
  const [hookId, setHookId] = useState("");
  const [tz, setTz] = useState("");
  const [prompt, setPrompt] = useState("");
  const [isolated, setIsolated] = useState(false);
  const [command, setCommand] = useState("");
  const [cwd, setCwd] = useState("");
  const [timeoutSecs, setTimeoutSecs] = useState("");
  const [deliveryKind, setDeliveryKind] = useState<DeliveryKind>("chat");
  const [webhookUrl, setWebhookUrl] = useState("");
  const [webhookToken, setWebhookToken] = useState("");
  const [notifierIntegrationId, setNotifierIntegrationId] = useState("");
  const [notifierTarget, setNotifierTarget] = useState("");
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

  const handleTriggerKindChange = (value: string) => {
    if (!isTriggerKind(value)) return;
    setTriggerKind(value);
    setLocalError(null);
  };

  const handleActionKindChange = (value: string) => {
    if (!isActionKind(value)) return;
    setActionKind(value);
    setLocalError(null);
  };

  const handleDeliveryKindChange = (value: string) => {
    if (!isDeliveryKind(value)) return;
    setDeliveryKind(value);
    setLocalError(null);
  };

  const validateTrigger = (): string | null => {
    if (triggerKind === "interval") return validateInterval(every);
    if (triggerKind === "once") return validateOneShot(at);
    if (triggerKind === "webhook") {
      if (!hookId.trim()) return "Webhook hook ID is required.";
      return null;
    }
    return validateCron(cron);
  };

  const validateAction = (): string | null => {
    if (actionKind === "agent") {
      if (!prompt.trim()) return "Prompt is required.";
      return null;
    }
    if (!command.trim()) return "Command is required.";
    if (timeoutSecs.trim()) {
      const timeout = parseTimeout(timeoutSecs);
      if (
        !POSITIVE_INTEGER_PATTERN.test(timeoutSecs.trim()) ||
        timeout === undefined ||
        timeout <= 0
      ) {
        return "Timeout must be a positive number of seconds.";
      }
    }
    return null;
  };

  const validateDelivery = (): string | null => {
    if (deliveryKind === "chat") return null;
    if (actionKind !== "command") {
      return "Webhook and notifier delivery require a command action.";
    }
    if (deliveryKind === "webhook") {
      if (!webhookUrl.trim()) return "Webhook URL is required.";
      return null;
    }
    if (!notifierIntegrationId.trim()) {
      return "Notifier integration ID is required.";
    }
    return null;
  };

  const buildTriggerRequest = (): Pick<
    CreateCronRequest,
    "cron" | "every" | "at" | "tz" | "recurring" | "trigger"
  > => {
    if (triggerKind === "interval") {
      return { every: every.trim(), recurring };
    }
    if (triggerKind === "once") {
      return { at: at.trim(), recurring: false };
    }
    if (triggerKind === "webhook") {
      return { trigger: { kind: "webhook", hook_id: hookId.trim() } };
    }
    return {
      cron: cron.trim(),
      recurring,
      ...(tz.trim() ? { tz: tz.trim() } : {}),
    };
  };

  const buildActionRequest = (): Pick<
    CreateCronRequest,
    "prompt" | "isolated" | "command" | "cwd" | "timeout_secs"
  > => {
    if (actionKind === "agent") {
      return {
        prompt: prompt.trim(),
        ...(isolated ? { isolated } : {}),
      };
    }
    return {
      command: command.trim(),
      ...(cwd.trim() ? { cwd: cwd.trim() } : {}),
      ...(timeoutSecs.trim()
        ? { timeout_secs: Number(timeoutSecs.trim()) }
        : {}),
    };
  };

  const buildDeliveryRequest = (): Pick<CreateCronRequest, "delivery"> => {
    if (deliveryKind === "chat") return {};
    if (deliveryKind === "webhook") {
      return {
        delivery: {
          kind: "webhook",
          url: webhookUrl.trim(),
          ...(webhookToken.trim() ? { token: webhookToken.trim() } : {}),
        },
      };
    }
    return {
      delivery: {
        kind: "notifier",
        integration_id: notifierIntegrationId.trim(),
        ...(notifierTarget.trim() ? { target: notifierTarget.trim() } : {}),
      },
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
    const triggerError = validateTrigger();
    if (triggerError) {
      setLocalError(triggerError);
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
    const actionError = validateAction();
    if (actionError) {
      setLocalError(actionError);
      return;
    }
    const deliveryError = validateDelivery();
    if (deliveryError) {
      setLocalError(deliveryError);
      return;
    }

    setLocalError(null);
    await onSubmit({
      ...buildTriggerRequest(),
      ...buildActionRequest(),
      ...buildDeliveryRequest(),
      durable,
      description: description.trim(),
    });
  };

  const submitForm = (event: FormEvent<HTMLFormElement>) => {
    void handleSubmit(event);
  };

  const errorMessage = localError ?? backendError;
  const showRecurring = triggerKind !== "once" && triggerKind !== "webhook";

  return (
    <form className={styles.form} onSubmit={submitForm}>
      <p className={styles.sectionHint}>
        Build a job from one trigger, one action, and one delivery target. Cron,
        interval, and one-shot triggers run on time; webhook triggers wait for a
        daemon hook call.
      </p>

      <fieldset className={styles.builderSection}>
        <legend className={styles.builderLegend}>Trigger</legend>
        <div className={styles.builderSectionContent}>
          <Field label="Trigger kind">
            <SegmentedControl
              aria-label="Trigger kind"
              name="scheduler-trigger-kind"
              options={TRIGGER_OPTIONS}
              value={triggerKind}
              onValueChange={handleTriggerKindChange}
            />
          </Field>

          {triggerKind === "cron" ? (
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

              <Field
                label="Timezone"
                helper="Optional IANA timezone, such as UTC."
              >
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

          {triggerKind === "interval" ? (
            <Field
              label="Interval"
              helper="Use s, m, h, or d suffixes."
              required
            >
              <FieldText
                className={styles.monoField}
                value={every}
                onChange={setEvery}
                aria-label="Interval"
                placeholder="30m"
              />
            </Field>
          ) : null}

          {triggerKind === "once" ? (
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

          {triggerKind === "webhook" ? (
            <Field
              label="Hook ID"
              helper="Matches the daemon /hooks/:name route that fires this job."
              required
            >
              <FieldText
                className={styles.monoField}
                value={hookId}
                onChange={setHookId}
                aria-label="Hook ID"
                placeholder="deploy"
              />
            </Field>
          ) : null}

          <Field
            label="Description"
            helper={`${description.length}/80`}
            required
          >
            <FieldText
              className={styles.fieldControl}
              value={description}
              maxLength={80}
              onChange={setDescription}
              aria-label="Description"
            />
          </Field>

          <div className={styles.toggles}>
            {showRecurring ? (
              <Field label="Recurring" helper="Run on every matching schedule.">
                <FieldSwitch
                  checked={recurring}
                  onChange={setRecurring}
                  aria-label="Recurring"
                />
              </Field>
            ) : null}
            <Field
              label="Durable"
              helper="Persist this schedule for the project."
            >
              <FieldSwitch
                checked={durable}
                onChange={setDurable}
                aria-label="Durable"
              />
            </Field>
          </div>
        </div>
      </fieldset>

      <fieldset className={styles.builderSection}>
        <legend className={styles.builderLegend}>Action</legend>
        <div className={styles.builderSectionContent}>
          <Field label="Action">
            <SegmentedControl
              aria-label="Action"
              name="scheduler-action-kind"
              options={ACTION_OPTIONS}
              value={actionKind}
              onValueChange={handleActionKindChange}
            />
          </Field>

          {actionKind === "agent" ? (
            <>
              <Field label="Prompt" required>
                <FieldTextarea
                  className={styles.fieldControl}
                  value={prompt}
                  onChange={setPrompt}
                  aria-label="Prompt"
                  rows={4}
                />
              </Field>
              <Field
                label="Isolated session"
                helper="Start a fresh session for each scheduled agent turn."
              >
                <FieldSwitch
                  checked={isolated}
                  onChange={setIsolated}
                  aria-label="Isolated session"
                />
              </Field>
            </>
          ) : null}

          {actionKind === "command" ? (
            <>
              <Field
                label="Command"
                helper="Runs without an agent turn and sends output to this chat."
                required
              >
                <FieldText
                  className={styles.monoField}
                  value={command}
                  onChange={setCommand}
                  aria-label="Command"
                  placeholder="npm test"
                />
              </Field>
              <div className={styles.commandGrid}>
                <Field
                  label="Working directory"
                  helper="Optional project path."
                >
                  <FieldText
                    className={styles.fieldControl}
                    value={cwd}
                    onChange={setCwd}
                    aria-label="Working directory"
                    placeholder="refact-agent/gui"
                  />
                </Field>
                <Field label="Timeout" helper="Optional seconds.">
                  <FieldText
                    className={styles.fieldControl}
                    inputMode="numeric"
                    value={timeoutSecs}
                    onChange={setTimeoutSecs}
                    aria-label="Timeout"
                    placeholder="600"
                  />
                </Field>
              </div>
            </>
          ) : null}
        </div>
      </fieldset>

      <fieldset className={styles.builderSection}>
        <legend className={styles.builderLegend}>Delivery</legend>
        <div className={styles.builderSectionContent}>
          <Field label="Delivery">
            <SegmentedControl
              aria-label="Delivery"
              name="scheduler-delivery-kind"
              options={DELIVERY_OPTIONS}
              value={deliveryKind}
              onValueChange={handleDeliveryKindChange}
            />
          </Field>

          {deliveryKind === "webhook" ? (
            <div className={styles.commandGrid}>
              <Field label="Webhook URL" required>
                <FieldText
                  className={styles.fieldControl}
                  value={webhookUrl}
                  onChange={setWebhookUrl}
                  aria-label="Webhook URL"
                  placeholder="https://example.com/scheduler"
                />
              </Field>
              <Field label="Webhook token" helper="Optional bearer token.">
                <FieldText
                  className={styles.fieldControl}
                  value={webhookToken}
                  onChange={setWebhookToken}
                  aria-label="Webhook token"
                  placeholder="Optional secret"
                />
              </Field>
            </div>
          ) : null}

          {deliveryKind === "notifier" ? (
            <div className={styles.commandGrid}>
              <Field label="Notifier integration ID" required>
                <FieldText
                  className={styles.fieldControl}
                  value={notifierIntegrationId}
                  onChange={setNotifierIntegrationId}
                  aria-label="Notifier integration ID"
                  placeholder="notifier_telegram"
                />
              </Field>
              <Field
                label="Notifier target"
                helper="Optional channel or chat id."
              >
                <FieldText
                  className={styles.fieldControl}
                  value={notifierTarget}
                  onChange={setNotifierTarget}
                  aria-label="Notifier target"
                  placeholder="chat-1"
                />
              </Field>
            </div>
          ) : null}
        </div>
      </fieldset>

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
