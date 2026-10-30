import { useEffect, useState } from "react";
import {
  Badge,
  Button,
  ButtonGroup,
  FieldError,
  FieldSelect,
  FieldSwitch,
  FieldText,
  LoadingState,
  SettingItem,
  Surface,
} from "../../components/ui";
import {
  type CoordinationSettingsScope,
  type TrajectorySettingValue,
  type TrajectorySettingsConfig,
  useGetCoordinationSettingsQuery,
  useSaveCoordinationSettingsMutation,
} from "../../services/refact/performance";
import { formatComponentName } from "./performanceFormatters";
import { settingsEqual } from "./settingPaths";
import { RootWeightsEditor } from "./RootWeightsEditor";
import styles from "./PerformancePage.module.css";

function errorText(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    if ("data" in error && typeof error.data === "string") return error.data;
    if ("message" in error && typeof error.message === "string")
      return error.message;
  }
  return "Could not save coordination settings.";
}

const recoveryLabels: Partial<Record<string, [string, string]>> = {
  recovery_auto_retry_enabled: [
    "Automatic model recovery",
    "Retry transient model errors automatically.",
  ],
  recovery_max_retries: [
    "Maximum new model requests",
    "Maximum actual model requests after a failure.",
  ],
  recovery_delay_ms: [
    "Recovery delay (milliseconds)",
    "Delay between automatic attempts.",
  ],
  recovery_total_timeout_ms: [
    "Recovery total time limit (milliseconds)",
    "Strict total time from the first failure; in-flight attempts are cancelled at the deadline.",
  ],
};

export function CoordinationSettingsPanel() {
  const [scope, setScope] = useState<CoordinationSettingsScope>("global");
  const {
    currentData: data,
    isFetching,
    error,
  } = useGetCoordinationSettingsQuery(scope);
  const [save, saveState] = useSaveCoordinationSettingsMutation();
  const [draft, setDraft] = useState<TrajectorySettingsConfig | null>(null);
  const [loaded, setLoaded] = useState<TrajectorySettingsConfig | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    setDraft(data?.config ?? null);
    setLoaded(data?.config ?? null);
    setSaveError(null);
    setSaved(false);
  }, [data, scope]);

  const change = (name: string, value: TrajectorySettingValue) => {
    setDraft((current) => (current ? { ...current, [name]: value } : current));
    setSaved(false);
    setSaveError(null);
  };
  const inherit = (name: string) => {
    setDraft((current) => {
      if (!current) return current;
      return Object.fromEntries(
        Object.entries(current).filter(([key]) => key !== name),
      );
    });
    setSaved(false);
  };
  const invalid =
    data?.fields.some((field) => {
      if (!draft || !(field.name in draft)) return false;
      const value = draft[field.name];
      if (field.value_type === "boolean") return typeof value !== "boolean";
      if (field.value_type === "integer_map") {
        if (typeof value !== "object" || value === null) return true;
        return Object.entries(value).some(
          ([key, weight]) =>
            key.trim() === "" ||
            key.trim() !== key ||
            typeof weight !== "number" ||
            !Number.isSafeInteger(weight) ||
            weight < (field.minimum ?? 1) ||
            (field.maximum != null && weight > field.maximum),
        );
      }
      return (
        typeof value !== "number" ||
        !Number.isSafeInteger(value) ||
        (field.minimum != null && value < field.minimum) ||
        (field.maximum != null && value > field.maximum)
      );
    }) ?? false;
  const hasChanges =
    draft !== null && loaded !== null && !settingsEqual(draft, loaded);

  const persist = async () => {
    if (!draft || invalid) return;
    setSaveError(null);
    try {
      const response = await save({ scope, config: draft }).unwrap();
      setDraft(response.config);
      setLoaded(response.config);
      setSaved(true);
    } catch (cause) {
      setSaveError(errorText(cause));
    }
  };

  return (
    <Surface className={styles.settingsPanel} variant="glass">
      <div className={styles.settingsHeader}>
        <div>
          <h2>Agent coordination</h2>
          <p>
            Shared execution capacity, work visibility and significant-change
            context updates. Goal effort budgets remain unlimited unless
            explicitly set.
          </p>
          <p>
            The background pool is shared by chat children and board agents;
            interactive top-level chats are not charged to it. Lowering a cap
            drains existing activity without stopping it. Root fairness quantum
            controls admissions per rotation; max resume burst bounds preference
            for returning work so waiting work still gets a turn.
          </p>
          <p>
            Launch context limits bound selected entries, messages, serialized
            bytes and estimated tokens, including parent instructions and source
            metadata. Source bytes bound each file read. Changes apply to new
            launch preparations; over-limit context is reported, not silently
            truncated.
          </p>
        </div>
        <FieldSelect
          aria-label="Coordination settings scope"
          value={scope}
          onChange={(value) => {
            if (value === "global" || value === "project") setScope(value);
          }}
          options={[
            { value: "global", label: "Global" },
            { value: "project", label: "This project" },
          ]}
          disabled={saveState.isLoading}
        />
      </div>
      {isFetching && !data ? (
        <LoadingState label="Loading coordination settings" />
      ) : null}
      {error ? (
        <FieldError>Could not load coordination settings.</FieldError>
      ) : null}
      {data && draft ? (
        <>
          {data.fields.map((field) => {
            const inherited = scope === "project" && !(field.name in draft);
            const value = inherited
              ? data.global[field.name]
              : draft[field.name];
            const recovery = recoveryLabels[field.name];
            const title = recovery?.[0] ?? formatComponentName(field.name);
            return (
              <SettingItem
                key={field.name}
                title={
                  <span className={styles.settingTitle}>
                    {title}
                    {inherited ? <Badge size="xs">Inherited</Badge> : null}
                  </span>
                }
                description={`${
                  recovery
                    ? `${recovery[1]} Changes apply to the next run. `
                    : ""
                }Default: ${
                  field.value_type === "integer_map"
                    ? "equal root weights"
                    : String(data.defaults[field.name])
                }. Applies ${
                  field.apply_mode === "live" ? "live" : "after restart"
                }.${
                  field.minimum != null && field.maximum != null
                    ? ` Range: ${field.minimum}–${field.maximum}.`
                    : ""
                }`}
                control={
                  <div className={styles.settingControl}>
                    {field.value_type === "integer_map" ? (
                      <RootWeightsEditor
                        value={value}
                        onChange={(next) => change(field.name, next)}
                        disabled={saveState.isLoading}
                        maximum={field.maximum ?? undefined}
                        maxEntries={
                          typeof data.effective.max_root_weight_entries ===
                          "number"
                            ? data.effective.max_root_weight_entries
                            : 0
                        }
                      />
                    ) : field.value_type === "boolean" ? (
                      <FieldSwitch
                        aria-label={title}
                        checked={value === true}
                        onChange={(next) => change(field.name, next)}
                        disabled={saveState.isLoading}
                      />
                    ) : (
                      <FieldText
                        aria-label={title}
                        type="number"
                        inputMode="numeric"
                        min={field.minimum ?? undefined}
                        max={field.maximum ?? undefined}
                        step={1}
                        value={typeof value === "number" ? String(value) : ""}
                        onChange={(next) =>
                          change(
                            field.name,
                            next.trim() === "" ? null : Number(next),
                          )
                        }
                        disabled={saveState.isLoading}
                      />
                    )}
                    {scope === "project" && !inherited ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => inherit(field.name)}
                        disabled={saveState.isLoading}
                      >
                        Use global
                      </Button>
                    ) : null}
                  </div>
                }
              />
            );
          })}
          <ButtonGroup>
            <Button
              variant="soft"
              size="sm"
              disabled={!hasChanges || saveState.isLoading}
              onClick={() => {
                setDraft(loaded);
                setSaveError(null);
                setSaved(false);
              }}
            >
              Revert
            </Button>
            <Button
              variant="soft"
              size="sm"
              disabled={saveState.isLoading}
              onClick={() => {
                setDraft(scope === "project" ? {} : data.defaults);
                setSaved(false);
              }}
            >
              {scope === "project"
                ? "Inherit all global settings"
                : "Reset coordination defaults"}
            </Button>
            <Button
              variant="primary"
              size="sm"
              loading={saveState.isLoading}
              disabled={!hasChanges || invalid}
              onClick={() => void persist()}
            >
              Save coordination settings
            </Button>
          </ButtonGroup>
          {invalid ? (
            <FieldError>
              Enter valid values within the displayed ranges.
            </FieldError>
          ) : null}
          {saveError ? <FieldError>{saveError}</FieldError> : null}
          {saved ? (
            <p role="status" className={styles.settingsSuccess}>
              Coordination settings saved.
            </p>
          ) : null}
        </>
      ) : null}
    </Surface>
  );
}
