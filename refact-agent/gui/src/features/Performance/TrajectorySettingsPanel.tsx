import { useEffect, useMemo, useState } from "react";

import {
  Badge,
  Button,
  ButtonGroup,
  FieldError,
  FieldSwitch,
  FieldText,
  LoadingState,
  SettingItem,
  Surface,
} from "../../components/ui";
import {
  type TrajectorySettingField,
  type TrajectorySettingValue,
  type TrajectorySettingsConfig,
  useGetTrajectorySettingsQuery,
  useSaveTrajectorySettingsMutation,
} from "../../services/refact/performance";
import { formatComponentName } from "./performanceFormatters";
import styles from "./PerformancePage.module.css";

type SettingsGroup = {
  title: string;
  description?: string;
  names: string[];
};

const SETTINGS_GROUPS: SettingsGroup[] = [
  {
    title: "Retention",
    names: [
      "internal_traces_keep_per_folder",
      "internal_trace_prune_interval_secs",
      "buddy_conversations_keep",
      "buddy_conversations_prune_interval_secs",
      "buddy_conversations_prune_min_age_secs",
    ],
  },
  {
    title: "Session lifecycle",
    names: [
      "session_idle_timeout_secs",
      "session_cleanup_interval_secs",
      "stream_idle_timeout_secs",
      "stream_total_timeout_secs",
    ],
  },
  {
    title: "Chat limits",
    names: [
      "max_queue_size",
      "event_channel_capacity",
      "recent_request_ids_capacity",
      "max_parallel_tools",
      "max_images_per_message",
      "max_file_size",
    ],
  },
  {
    title: "Enrichment caps",
    names: [
      "auto_enrichment_total_token_cap",
      "auto_enrichment_card_token_cap",
      "auto_enrichment_knowledge_top_n",
      "auto_enrichment_trajectory_top_n",
    ],
  },
  {
    title: "Performance optimizations",
    description:
      "These optimizations are enabled by default. Disabling one reduces performance; restart-required changes take effect after the next engine restart.",
    names: [
      "trajectory_writer_enabled",
      "trajectory_index_coordinator_enabled",
      "trajectory_watcher_self_write_enabled",
      "tool_catalog_snapshots_enabled",
      "vecdb_path_coalescing_enabled",
    ],
  },
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function errorMessage(error: unknown): string {
  if (!isRecord(error)) return "Could not save trajectory settings.";
  if (typeof error.error === "string") return error.error;
  if (typeof error.message === "string") return error.message;
  if (!isRecord(error.data)) {
    return typeof error.data === "string"
      ? error.data
      : "Could not save trajectory settings.";
  }
  if (typeof error.data.detail === "string") return error.data.detail;
  if (typeof error.data.message === "string") return error.data.message;
  if (typeof error.data.error === "string") return error.data.error;
  return "Could not save trajectory settings.";
}

function rangeHint(field: TrajectorySettingField): string | undefined {
  if (field.value_type !== "integer") return undefined;
  if (field.minimum == null || field.maximum == null) {
    return "Enter a whole number.";
  }
  return `Allowed range: ${field.minimum.toLocaleString()}–${field.maximum.toLocaleString()}.`;
}

function fieldError(
  field: TrajectorySettingField,
  value: TrajectorySettingValue,
  allowsNull: boolean,
): string | undefined {
  if (field.value_type === "boolean") {
    return typeof value === "boolean" ? undefined : "Enter true or false.";
  }
  if (field.value_type !== "integer") return undefined;
  if (value === null && allowsNull) return undefined;
  if (typeof value !== "number" || !Number.isInteger(value)) {
    return "Enter a whole number.";
  }
  if (field.minimum != null && value < field.minimum) {
    return rangeHint(field);
  }
  if (field.maximum != null && value > field.maximum) {
    return rangeHint(field);
  }
  return undefined;
}

function draftEquals(
  draft: TrajectorySettingsConfig,
  loaded: TrajectorySettingsConfig,
): boolean {
  const keys = new Set([...Object.keys(draft), ...Object.keys(loaded)]);
  return [...keys].every((key) => draft[key] === loaded[key]);
}

function parseInteger(value: string): number | null {
  if (!/^-?\d+$/.test(value.trim())) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function valueToInput(value: TrajectorySettingValue): string {
  return value === null ? "" : String(value);
}

function backendErrorForField(
  name: string,
  error: string | null,
): string | undefined {
  return error?.includes(name) ? error : undefined;
}

export function TrajectorySettingsPanel() {
  const { data, error, isFetching } = useGetTrajectorySettingsQuery(undefined, {
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const [saveSettings, saveState] = useSaveTrajectorySettingsMutation();
  const [draft, setDraft] = useState<TrajectorySettingsConfig | null>(null);
  const [loaded, setLoaded] = useState<TrajectorySettingsConfig | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (!data) return;
    setDraft(data.config);
    setLoaded(data.config);
    setSaveError(null);
    setSaved(false);
  }, [data]);

  const fields = useMemo(() => data?.fields ?? [], [data?.fields]);
  const validationErrors = useMemo(() => {
    if (!draft) return new Map<string, string>();
    return new Map(
      fields.flatMap((field) => {
        const validationError = fieldError(
          field,
          draft[field.name],
          data?.config[field.name] === null ||
            data?.defaults[field.name] === null,
        );
        return validationError ? [[field.name, validationError]] : [];
      }),
    );
  }, [data?.config, data?.defaults, draft, fields]);
  const unknownFields = useMemo(() => {
    const knownNames = new Set(SETTINGS_GROUPS.flatMap((group) => group.names));
    return fields.filter((field) => !knownNames.has(field.name));
  }, [fields]);
  const hasChanges =
    draft !== null && loaded !== null && !draftEquals(draft, loaded);
  const canSave =
    hasChanges && validationErrors.size === 0 && !saveState.isLoading;

  const updateValue = (name: string, value: TrajectorySettingValue) => {
    setDraft((current) => (current ? { ...current, [name]: value } : current));
    setSaveError(null);
    setSaved(false);
  };

  const revert = () => {
    if (!loaded) return;
    setDraft(loaded);
    setSaveError(null);
    setSaved(false);
  };

  const resetToDefaults = () => {
    if (!data) return;
    setDraft(data.defaults);
    setSaveError(null);
    setSaved(false);
  };

  const save = async () => {
    if (!draft || !canSave) return;
    setSaveError(null);
    setSaved(false);
    try {
      const response = await saveSettings(draft).unwrap();
      setDraft(response.config);
      setLoaded(response.config);
      setSaved(true);
    } catch (requestError) {
      setSaveError(errorMessage(requestError));
    }
  };

  const renderField = (field: TrajectorySettingField) => {
    if (!draft || !(field.name in draft)) return null;
    const value = draft[field.name];
    const localError = validationErrors.get(field.name);
    const backendError = backendErrorForField(field.name, saveError);
    const restartRequired = field.apply_mode === "restart_required";
    const optimization = SETTINGS_GROUPS[4].names.includes(field.name);
    const description = [
      rangeHint(field),
      `Default: ${valueToInput(data?.defaults[field.name] ?? null) || "—"}.`,
      `Runtime: ${valueToInput(data?.current[field.name] ?? null) || "—"}.`,
      optimization && value === false
        ? "Disabled optimization reduces performance."
        : undefined,
    ]
      .filter((item): item is string => item !== undefined)
      .join(" ");

    return (
      <SettingItem
        className="rf-enter"
        key={field.name}
        title={
          <span className={styles.settingTitle}>
            {formatComponentName(field.name)}
            {restartRequired ? (
              <Badge size="xs" tone="warning" variant="soft">
                Requires restart
              </Badge>
            ) : null}
          </span>
        }
        description={description}
        control={
          field.value_type === "boolean" ? (
            <div className={styles.settingControl}>
              <FieldSwitch
                aria-label={formatComponentName(field.name)}
                checked={value === true}
                disabled={saveState.isLoading}
                onChange={(nextValue) => updateValue(field.name, nextValue)}
              />
              {localError ?? backendError ? (
                <FieldError>{localError ?? backendError}</FieldError>
              ) : null}
            </div>
          ) : field.value_type === "integer" ? (
            <div className={styles.settingControl}>
              <FieldText
                aria-label={formatComponentName(field.name)}
                aria-invalid={
                  localError !== undefined || backendError !== undefined
                }
                disabled={saveState.isLoading}
                inputMode="numeric"
                max={field.maximum ?? undefined}
                min={field.minimum ?? undefined}
                step={1}
                type="number"
                value={value === null ? "" : String(value)}
                onChange={(nextValue) =>
                  updateValue(field.name, parseInteger(nextValue))
                }
              />
              {localError ?? backendError ? (
                <FieldError>{localError ?? backendError}</FieldError>
              ) : null}
            </div>
          ) : (
            <div className={styles.settingControl}>
              <FieldText
                aria-label={formatComponentName(field.name)}
                disabled={saveState.isLoading}
                value={valueToInput(value)}
                onChange={(nextValue) => updateValue(field.name, nextValue)}
              />
              {backendError ? <FieldError>{backendError}</FieldError> : null}
            </div>
          )
        }
      />
    );
  };

  if (isFetching && !draft) {
    return <LoadingState label="Loading trajectory settings" />;
  }

  if (!draft || !data) {
    return (
      <Surface className={styles.settingsPanel} variant="glass">
        <h2>Trajectory settings</h2>
        <FieldError>
          Could not load trajectory settings: {errorMessage(error)}
        </FieldError>
      </Surface>
    );
  }

  return (
    <Surface className={styles.settingsPanel} variant="glass">
      <div className={styles.settingsHeader}>
        <div>
          <h2>Trajectory and chat settings</h2>
          <p>
            Settings are loaded from this engine, including their defaults,
            valid ranges, and apply mode.
          </p>
        </div>
        <ButtonGroup>
          <Button
            disabled={!hasChanges || saveState.isLoading}
            onClick={revert}
            size="sm"
            variant="soft"
          >
            Revert to loaded
          </Button>
          <Button
            disabled={saveState.isLoading}
            onClick={resetToDefaults}
            size="sm"
            variant="soft"
          >
            Reset to defaults
          </Button>
          <Button
            disabled={!canSave}
            loading={saveState.isLoading}
            onClick={() => void save()}
            size="sm"
            variant="primary"
          >
            Save settings
          </Button>
        </ButtonGroup>
      </div>

      {SETTINGS_GROUPS.map((group) => {
        const groupFields = group.names
          .map((name) => fields.find((field) => field.name === name))
          .filter(
            (field): field is TrajectorySettingField => field !== undefined,
          );
        if (groupFields.length === 0) return null;
        return (
          <section className={styles.settingsGroup} key={group.title}>
            <h3>{group.title}</h3>
            {group.description ? <p>{group.description}</p> : null}
            <div>{groupFields.map(renderField)}</div>
          </section>
        );
      })}

      {unknownFields.length > 0 ? (
        <section className={styles.settingsGroup}>
          <h3>Additional settings</h3>
          <div>{unknownFields.map(renderField)}</div>
        </section>
      ) : null}

      {data.environment_precedence ? (
        <p className={styles.settingsNote}>{data.environment_precedence}</p>
      ) : null}
      {saveError && !fields.some((field) => saveError.includes(field.name)) ? (
        <FieldError>{saveError}</FieldError>
      ) : null}
      {saved ? (
        <p className={styles.settingsSuccess}>Trajectory settings saved.</p>
      ) : null}
    </Surface>
  );
}
