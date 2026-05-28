import React, { FormEvent, useMemo, useState } from "react";
import {
  Button,
  Card,
  Checkbox,
  Flex,
  Select,
  Text,
  TextArea,
  TextField,
} from "@radix-ui/themes";
import {
  type CreateCronRequest,
  schedulerErrorMessage,
} from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type CronPreset = "hourly" | "daily" | "weekdays" | "five-min" | "custom";

type CronCreateFormProps = {
  onSubmit: (request: CreateCronRequest) => Promise<void>;
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

  const setSelectedPreset = (value: string) => {
    const nextPreset = value as CronPreset;
    setPreset(nextPreset);
    if (nextPreset !== "custom") {
      setCron(PRESETS[nextPreset]);
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

  return (
    <Card>
      <form className={styles.form} onSubmit={submitForm}>
        <Text size="4" weight="bold">
          Create schedule
        </Text>
        <div className={styles.inlineFields}>
          <label className={styles.field}>
            <Text size="2" weight="medium">
              Cron expression
            </Text>
            <TextField.Root
              value={cron}
              onChange={(event) => handleCronChange(event.target.value)}
              aria-label="Cron expression"
            />
          </label>
          <label className={styles.field}>
            <Text size="2" weight="medium">
              Preset
            </Text>
            <Select.Root value={preset} onValueChange={setSelectedPreset}>
              <Select.Trigger aria-label="Cron preset" />
              <Select.Content>
                <Select.Item value="hourly">Hourly</Select.Item>
                <Select.Item value="daily">Daily 9am</Select.Item>
                <Select.Item value="weekdays">Weekdays 9am</Select.Item>
                <Select.Item value="five-min">Every 5 min</Select.Item>
                <Select.Item value="custom">Custom</Select.Item>
              </Select.Content>
            </Select.Root>
          </label>
        </div>
        <label className={styles.field}>
          <Text size="2" weight="medium">
            Description
          </Text>
          <TextField.Root
            value={description}
            maxLength={80}
            onChange={(event) => setDescription(event.target.value)}
            aria-label="Description"
          />
          <Text size="1" color={description.length > 80 ? "red" : "gray"}>
            {description.length}/80
          </Text>
        </label>
        <label className={styles.field}>
          <Text size="2" weight="medium">
            Prompt
          </Text>
          <TextArea
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            aria-label="Prompt"
            rows={4}
          />
        </label>
        <div className={styles.toggles}>
          <label className={styles.toggle}>
            <Checkbox
              checked={recurring}
              onCheckedChange={(checked) => setRecurring(checked === true)}
            />
            <Text size="2">Recurring</Text>
          </label>
          <label className={styles.toggle}>
            <Checkbox
              checked={durable}
              onCheckedChange={(checked) => setDurable(checked === true)}
            />
            <Text size="2">Durable</Text>
          </label>
        </div>
        {(localError ?? backendError) && (
          <Text className={styles.error} role="alert" size="2">
            {localError ?? backendError}
          </Text>
        )}
        <Flex justify="end">
          <Button type="submit" disabled={isLoading || capExceeded}>
            {isLoading ? "Creating…" : "Create"}
          </Button>
        </Flex>
      </form>
    </Card>
  );
};
