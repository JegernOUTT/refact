import React, { useCallback, useEffect, useMemo, useState } from "react";

import { SchemaField } from "./SchemaField";
import { ProviderOAuth } from "./ProviderOAuth";
import { Spinner } from "../../../components/Spinner";

import { useProviderForm } from "./useProviderForm";
import type {
  ProviderListItem,
  ProviderStatus,
  ClaudeCodeUsageWindow,
  OpenAICodexUsageWindow,
  ModelTypeDefaults,
  ProviderDefaults,
} from "../../../services/refact";
import { ModelSelector } from "../../../components/Chat/ModelSelector";
import { Badge, Button, SettingItem, Surface } from "../../../components/ui";

import styles from "./ProviderForm.module.css";
import { ProviderModelsList } from "./ProviderModelsList/ProviderModelsList";
import {
  useGetOpenRouterHealthQuery,
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  useGetDefaultsQuery,
  useGetCapsQuery,
  useUpdateDefaultsMutation,
} from "../../../services/refact";

export type ProviderFormProps = {
  currentProvider: ProviderListItem;
};

export type { ProviderListItem };

const StatusBadge: React.FC<{ status: ProviderStatus }> = ({ status }) => {
  switch (status) {
    case "active":
      return <Badge tone="success">Active</Badge>;
    case "configured":
      return <Badge tone="warning">Configured</Badge>;
    case "not_configured":
      return <Badge tone="muted">Not configured</Badge>;
    default:
      return null;
  }
};

const UsageBar: React.FC<{ pct: number }> = ({ pct }) => (
  <progress
    className={styles.usageBar}
    max={100}
    value={pct}
    aria-label={`${Math.round(pct)}% used`}
  />
);

const ClaudeWindowRow: React.FC<{
  label: string;
  w: ClaudeCodeUsageWindow;
}> = ({ label, w }) => {
  const pct = Math.max(0, Math.min(w.percent_used, 100));
  const d = w.resets_at ? new Date(w.resets_at) : null;
  const resetText =
    d && !isNaN(d.getTime())
      ? `Resets ${d.toLocaleString(undefined, {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        })}`
      : null;
  return (
    <div className={styles.usageRow}>
      <div className={styles.usageRowHeader}>
        <span>{label}</span>
        <span>
          {Math.round(pct)}% used{resetText ? ` · ${resetText}` : ""}
        </span>
      </div>
      <UsageBar pct={pct} />
    </div>
  );
};

const CodexWindowRow: React.FC<{
  label: string;
  w: OpenAICodexUsageWindow;
  limitReached?: boolean;
}> = ({ label, w, limitReached }) => {
  const pct = Math.max(0, Math.min(w.used_percent, 100));
  const d = w.reset_at ? new Date(w.reset_at) : null;
  const resetText =
    d && !isNaN(d.getTime())
      ? `Resets ${d.toLocaleString(undefined, {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        })}`
      : null;
  return (
    <div className={styles.usageRow}>
      <div className={styles.usageRowHeader}>
        <span className={styles.usageLabelGroup}>
          {label}
          {limitReached ? <Badge tone="danger">Limit reached</Badge> : null}
        </span>
        <span>
          {Math.round(pct)}% used{resetText ? ` · ${resetText}` : ""}
        </span>
      </div>
      <UsageBar pct={pct} />
    </div>
  );
};

type DefaultModelKey =
  | "chat"
  | "chat_model_2"
  | "task_planner_agent_model"
  | "chat_light"
  | "chat_thinking"
  | "chat_buddy";

const DEFAULT_MODEL_FIELDS: {
  key: DefaultModelKey;
  label: string;
  description: string;
}[] = [
  {
    key: "chat",
    label: "Default chat",
    description: "Primary model for normal conversations.",
  },
  {
    key: "chat_model_2",
    label: "Chat model 2",
    description: "Secondary chat model slot for future chat workflows.",
  },
  {
    key: "task_planner_agent_model",
    label: "Task planner agent",
    description: "Model used by task management when spawning task agents.",
  },
  {
    key: "chat_light",
    label: "Light",
    description: "Fast model used by quick subagents and gathering steps.",
  },
  {
    key: "chat_thinking",
    label: "Thinking",
    description: "Reasoning model used by planning, review, and research.",
  },
  {
    key: "chat_buddy",
    label: "Companion",
    description: "Background companion model.",
  },
];

function normalizeProviderDefaults(
  defaults: ProviderDefaults | undefined,
): ProviderDefaults {
  return {
    chat: defaults?.chat ?? {},
    chat_model_2: defaults?.chat_model_2 ?? {},
    task_planner_agent_model: defaults?.task_planner_agent_model ?? {},
    chat_light: defaults?.chat_light ?? {},
    chat_thinking: defaults?.chat_thinking ?? {},
    chat_buddy: defaults?.chat_buddy ?? {},
    completion_model: defaults?.completion_model,
    embedding_model: defaults?.embedding_model,
  };
}

const ProviderDefaultModelsSetup: React.FC = () => {
  const {
    data: defaults,
    isLoading,
    isError,
    refetch,
  } = useGetDefaultsQuery(undefined);
  const { data: caps, refetch: refetchCaps } = useGetCapsQuery(undefined);
  const [updateDefaults, { isLoading: isSaving }] = useUpdateDefaultsMutation();
  const [localDefaults, setLocalDefaults] = useState<ProviderDefaults>(() =>
    normalizeProviderDefaults(undefined),
  );
  const [hasChanges, setHasChanges] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!defaults) return;
    setLocalDefaults(normalizeProviderDefaults(defaults));
    setHasChanges(false);
    setSaveError(null);
  }, [defaults]);

  const capsDefaults = useMemo(
    () => ({
      chat: caps?.chat_default_model ?? "",
      chat_model_2: caps?.chat_model_2 ?? "",
      task_planner_agent_model: caps?.task_planner_agent_model ?? "",
      chat_light: caps?.chat_light_model ?? "",
      chat_thinking: caps?.chat_thinking_model ?? "",
      chat_buddy: caps?.chat_buddy_model ?? "",
    }),
    [caps],
  );

  const handleModelChange = useCallback(
    (key: DefaultModelKey, model: string) => {
      setLocalDefaults((prev) => ({
        ...prev,
        [key]: { ...(prev[key] ?? {}), model } as ModelTypeDefaults,
      }));
      setHasChanges(true);
      setSaveError(null);
    },
    [],
  );

  const handleSave = useCallback(async () => {
    try {
      await updateDefaults(localDefaults).unwrap();
      setHasChanges(false);
      setSaveError(null);
      void refetch();
      void refetchCaps();
    } catch {
      setSaveError("Failed to save default models.");
    }
  }, [localDefaults, refetch, refetchCaps, updateDefaults]);

  if (isError) return null;

  return (
    <Surface className={styles.defaultsPanel} variant="surface-1">
      <div className={styles.defaultHeader}>
        <div>
          <div className={styles.defaultTitle}>Global default models</div>
          <div className={styles.defaultDescription}>
            These defaults apply across all providers. Enable provider models
            above, then choose which model type each feature should use. Empty
            slots stay unset.
          </div>
        </div>
        <Button
          variant="primary"
          size="sm"
          onClick={() => void handleSave()}
          disabled={!hasChanges || isSaving || isLoading}
        >
          {isSaving ? "Saving..." : "Save"}
        </Button>
      </div>
      {saveError ? <div className={styles.errorText}>{saveError}</div> : null}
      <div className={styles.defaultRows}>
        {DEFAULT_MODEL_FIELDS.map(({ key, label, description }) => (
          <SettingItem
            key={key}
            title={label}
            description={description}
            layout="stack"
          >
            <ModelSelector
              value={localDefaults[key]?.model}
              onValueChange={(model) => handleModelChange(key, model)}
              defaultValue={capsDefaults[key]}
              showLabel={false}
              compact={false}
              allowUnset
              unsetLabel="None"
              disabled={isLoading || isSaving}
            />
          </SettingItem>
        ))}
      </div>
    </Surface>
  );
};

export const ProviderForm: React.FC<ProviderFormProps> = ({
  currentProvider,
}) => {
  const baseProvider = currentProvider.base_provider;
  const { data: openRouterHealth } = useGetOpenRouterHealthQuery(
    { providerName: currentProvider.name, useInstanceRoute: true },
    { skip: baseProvider !== "openrouter" },
  );
  const { data: claudeUsage, isError: claudeUsageError } =
    useGetClaudeCodeUsageQuery(
      { providerName: currentProvider.name, useInstanceRoute: true },
      { skip: baseProvider !== "claude_code", pollingInterval: 60_000 },
    );
  const { data: codexUsage, isError: codexUsageError } =
    useGetOpenAICodexUsageQuery(
      { providerName: currentProvider.name, useInstanceRoute: true },
      { skip: baseProvider !== "openai_codex", pollingInterval: 60_000 },
    );
  const {
    areShowingExtraFields,
    formValues,
    parsedSchema,
    importantFields,
    extraFields,
    isProviderLoadedSuccessfully,
    setAreShowingExtraFields,
    handleFieldSave,
    detailedProvider,
  } = useProviderForm({ providerName: currentProvider.name });

  if (!isProviderLoadedSuccessfully || !formValues || !parsedSchema) {
    return <Spinner spinning />;
  }

  const hasOAuth = parsedSchema.oauth?.supported === true;
  const status: ProviderStatus =
    detailedProvider?.status ?? currentProvider.status;
  const hasCredentials =
    detailedProvider?.has_credentials ?? currentProvider.has_credentials;
  const isReadonly = formValues.readonly;

  return (
    <div className={styles.providerForm}>
      <div className={styles.formSection}>
        <div className={styles.statusRow}>
          <StatusBadge status={status} />
          {baseProvider === "openrouter" && openRouterHealth ? (
            <Badge tone={openRouterHealth.ok ? "success" : "danger"}>
              {openRouterHealth.ok ? "Key OK" : "Key Error"}
            </Badge>
          ) : null}
          {parsedSchema.description ? (
            <div className={styles.providerDescription}>
              {parsedSchema.description.trim().split("\n")[0]}
            </div>
          ) : null}
        </div>

        {claudeUsage?.data && !claudeUsage.error ? (
          <Surface className={styles.usagePanel} variant="plain">
            <div className={styles.usageTitle}>Usage</div>
            <div className={styles.usageRows}>
              {claudeUsage.data.five_hour ? (
                <ClaudeWindowRow
                  label="Session (5 hour)"
                  w={claudeUsage.data.five_hour}
                />
              ) : null}
              {claudeUsage.data.seven_day ? (
                <ClaudeWindowRow
                  label="Weekly"
                  w={claudeUsage.data.seven_day}
                />
              ) : null}
              {claudeUsage.data.extra_usage ? (
                <div className={styles.usageRow}>
                  <div className={styles.usageRowHeader}>
                    <span>Extra usage</span>
                    <span>
                      {claudeUsage.data.extra_usage.is_enabled
                        ? "enabled"
                        : "disabled"}
                      {" · $"}
                      {claudeUsage.data.extra_usage.used_credits.toFixed(
                        2,
                      )}{" "}
                      spent
                      {typeof claudeUsage.data.extra_usage.monthly_limit ===
                      "number"
                        ? ` / $${claudeUsage.data.extra_usage.monthly_limit.toFixed(
                            0,
                          )} limit`
                        : " / unlimited"}
                    </span>
                  </div>
                  {typeof claudeUsage.data.extra_usage.utilization ===
                  "number" ? (
                    <UsageBar
                      pct={Math.max(
                        0,
                        Math.min(claudeUsage.data.extra_usage.utilization, 100),
                      )}
                    />
                  ) : null}
                </div>
              ) : null}
            </div>
          </Surface>
        ) : null}
        {claudeUsage?.error != null || claudeUsageError ? (
          <div className={styles.defaultDescription}>
            Usage: {claudeUsage?.error ?? "Failed to load"}
          </div>
        ) : null}

        {codexUsage?.data && !codexUsage.error ? (
          <Surface className={styles.usagePanel} variant="plain">
            <div className={styles.usageHeader}>
              <div className={styles.usageTitle}>Usage</div>
              {codexUsage.data.plan_type ? (
                <Badge tone="accent">{codexUsage.data.plan_type}</Badge>
              ) : null}
            </div>
            <div className={styles.usageRows}>
              {codexUsage.data.rate_limit?.primary_window ? (
                <CodexWindowRow
                  label="Session (5 hour)"
                  w={codexUsage.data.rate_limit.primary_window}
                  limitReached={codexUsage.data.rate_limit.limit_reached}
                />
              ) : null}
              {codexUsage.data.rate_limit?.secondary_window ? (
                <CodexWindowRow
                  label="Weekly"
                  w={codexUsage.data.rate_limit.secondary_window}
                />
              ) : null}
              {codexUsage.data.code_review_rate_limit?.primary_window ? (
                <CodexWindowRow
                  label="Code review (weekly)"
                  w={codexUsage.data.code_review_rate_limit.primary_window}
                  limitReached={
                    codexUsage.data.code_review_rate_limit.limit_reached
                  }
                />
              ) : null}
              {codexUsage.data.credits ? (
                <div className={styles.defaultDescription}>
                  Credits:{" "}
                  {codexUsage.data.credits.unlimited
                    ? "unlimited"
                    : codexUsage.data.credits.has_credits
                      ? `${codexUsage.data.credits.balance} remaining`
                      : "none"}
                </div>
              ) : null}
            </div>
          </Surface>
        ) : null}
        {codexUsage?.error != null || codexUsageError ? (
          <div className={styles.defaultDescription}>
            Usage: {codexUsage?.error ?? "Failed to load"}
          </div>
        ) : null}

        <div className={styles.formSection}>
          {hasOAuth ? (
            <>
              <ProviderOAuth
                providerName={currentProvider.name}
                baseProvider={baseProvider}
                oauthConnected={Boolean(
                  "oauth_connected" in formValues && formValues.oauth_connected,
                )}
                authStatus={
                  "auth_status" in formValues
                    ? String(formValues.auth_status)
                    : ""
                }
              />
            </>
          ) : null}

          <div className={styles.formFields}>
            {importantFields.map((field) => (
              <SchemaField
                key={field.key}
                field={field}
                value={formValues[field.key]}
                disabled={isReadonly}
                onSave={handleFieldSave}
              />
            ))}
          </div>

          {extraFields.length > 0 ? (
            <>
              <div className={styles.advancedToggleWrap}>
                <Button
                  className={styles.extraButton}
                  variant="ghost"
                  size="sm"
                  onClick={() => setAreShowingExtraFields((prev) => !prev)}
                >
                  {areShowingExtraFields ? "Hide" : "Show"} advanced fields
                </Button>
              </div>

              {areShowingExtraFields ? (
                <div className={styles.formFields}>
                  {extraFields.map((field) => (
                    <SchemaField
                      key={field.key}
                      field={field}
                      value={formValues[field.key]}
                      disabled={isReadonly}
                      onSave={handleFieldSave}
                    />
                  ))}
                </div>
              ) : null}
            </>
          ) : null}
        </div>

        {hasCredentials ? (
          <ProviderModelsList provider={currentProvider} />
        ) : null}
        {hasCredentials ? <ProviderDefaultModelsSetup /> : null}
      </div>
    </div>
  );
};
