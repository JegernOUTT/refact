import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Badge, Button, Card, Flex, Separator, Text } from "@radix-ui/themes";

import { SchemaField } from "./SchemaField";
import { ProviderOAuth } from "./ProviderOAuth";
import { RoleSeparatedModelConfig } from "./RoleSeparatedModelConfig";
import { Spinner } from "../../../components/Spinner";

import { useProviderForm } from "./useProviderForm";
import type {
  ProviderListItem,
  ProviderStatus,
  ClaudeCodeUsageWindow,
  OpenAICodexUsageWindow,
  ProviderDefaults,
} from "../../../services/refact";
import { ModelSelector } from "../../../components/Chat/ModelSelector";

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
      return (
        <Badge color="green" size="1">
          Active
        </Badge>
      );
    case "configured":
      return (
        <Badge color="orange" size="1">
          Configured
        </Badge>
      );
    case "not_configured":
      return (
        <Badge color="gray" size="1">
          Not configured
        </Badge>
      );
    default:
      return null;
  }
};

const UsageBar: React.FC<{ pct: number }> = ({ pct }) => {
  const color =
    pct >= 90
      ? "var(--red-9)"
      : pct >= 70
        ? "var(--orange-9)"
        : "var(--green-9)";
  return (
    <div
      style={{
        height: "4px",
        width: "100%",
        borderRadius: "2px",
        background: "var(--gray-a4)",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          height: "100%",
          width: `${pct}%`,
          borderRadius: "2px",
          background: color,
          transition: "width 0.3s ease",
        }}
      />
    </div>
  );
};

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
    <Flex direction="column" gap="1">
      <Flex justify="between">
        <Text size="1" color="gray">
          {label}
        </Text>
        <Text size="1" color="gray">
          {Math.round(pct)}% used{resetText ? ` · ${resetText}` : ""}
        </Text>
      </Flex>
      <UsageBar pct={pct} />
    </Flex>
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
    <Flex direction="column" gap="1">
      <Flex justify="between" align="center">
        <Flex align="center" gap="1">
          <Text size="1" color="gray">
            {label}
          </Text>
          {limitReached && (
            <Badge color="red" size="1">
              Limit reached
            </Badge>
          )}
        </Flex>
        <Text size="1" color="gray">
          {Math.round(pct)}% used{resetText ? ` · ${resetText}` : ""}
        </Text>
      </Flex>
      <UsageBar pct={pct} />
    </Flex>
  );
};

type DefaultModelKey =
  | "chat"
  | "chat_model_2"
  | "task_planner_agent_model"
  | "chat_light"
  | "chat_thinking"
  | "chat_buddy"
  | "completion_model"
  | "embedding_model";

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
  {
    key: "completion_model",
    label: "Completion",
    description: "Default model for code completion.",
  },
  {
    key: "embedding_model",
    label: "Embedding",
    description: "Default model for semantic embeddings.",
  },
];

export function normalizeProviderDefaults(
  defaults: ProviderDefaults | undefined,
): ProviderDefaults {
  return {
    ...defaults,
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

function getModelCapability(key: DefaultModelKey) {
  if (key === "completion_model") return "completion";
  if (key === "embedding_model") return "embedding";
  return "chat";
}

function getDefaultModelValue(
  defaults: ProviderDefaults,
  key: DefaultModelKey,
): string | undefined {
  const value = defaults[key];
  if (typeof value === "string") return value;
  return value?.model;
}

function updateDefaultModelValue(
  defaults: ProviderDefaults,
  key: DefaultModelKey,
  model: string,
): ProviderDefaults {
  if (key === "completion_model" || key === "embedding_model") {
    return { ...defaults, [key]: model };
  }
  return { ...defaults, [key]: { ...(defaults[key] ?? {}), model } };
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
      completion_model: caps?.completion_default_model ?? "",
      embedding_model: caps?.embedding_model?.id ?? "",
    }),
    [caps],
  );

  const handleModelChange = useCallback(
    (key: DefaultModelKey, model: string) => {
      setLocalDefaults((prev) => updateDefaultModelValue(prev, key, model));
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
    <Card size="2" className={styles.defaultsCard}>
      <Flex direction="column" gap="3">
        <Flex justify="between" align="center" gap="3">
          <Flex direction="column" gap="1">
            <Text size="2" weight="medium">
              Global default models
            </Text>
            <Text size="1" color="gray">
              These defaults apply across all providers. Enable provider models
              above, then choose which model type each feature should use. Empty
              slots stay unset.
            </Text>
          </Flex>
          <Button
            size="1"
            variant="solid"
            onClick={() => void handleSave()}
            disabled={!hasChanges || isSaving || isLoading}
          >
            {isSaving ? "Saving..." : "Save"}
          </Button>
        </Flex>

        {saveError && (
          <Text size="1" color="red">
            {saveError}
          </Text>
        )}

        <Flex direction="column" gap="3">
          {DEFAULT_MODEL_FIELDS.map(({ key, label, description }) => (
            <Flex key={key} direction="column" gap="1">
              <Flex justify="between" align="baseline" gap="3">
                <Text size="1" weight="medium">
                  {label}
                </Text>
                <Text size="1" color="gray">
                  {description}
                </Text>
              </Flex>
              <ModelSelector
                value={getDefaultModelValue(localDefaults, key)}
                onValueChange={(model) => handleModelChange(key, model)}
                defaultValue={capsDefaults[key]}
                showLabel={false}
                compact={false}
                allowUnset
                unsetLabel="None"
                capability={getModelCapability(key)}
                disabled={isLoading || isSaving}
              />
            </Flex>
          ))}
        </Flex>
      </Flex>
    </Card>
  );
};

export const ProviderForm: React.FC<ProviderFormProps> = ({
  currentProvider,
}) => {
  const baseProvider = currentProvider.base_provider;
  const { data: openRouterHealth } = useGetOpenRouterHealthQuery(
    { providerName: currentProvider.name, useInstanceRoute: true },
    {
      skip: baseProvider !== "openrouter",
    },
  );
  const { data: claudeUsage, isError: claudeUsageError } =
    useGetClaudeCodeUsageQuery(
      { providerName: currentProvider.name, useInstanceRoute: true },
      {
        skip: baseProvider !== "claude_code",
        pollingInterval: 60_000,
      },
    );
  const { data: codexUsage, isError: codexUsageError } =
    useGetOpenAICodexUsageQuery(
      { providerName: currentProvider.name, useInstanceRoute: true },
      {
        skip: baseProvider !== "openai_codex",
        pollingInterval: 60_000,
      },
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
    <Flex
      direction="column"
      width="100%"
      minHeight="100%"
      mt="2"
      pb="4"
      gap="3"
    >
      <Flex align="center" gap="2">
        <StatusBadge status={status} />
        {baseProvider === "openrouter" && openRouterHealth && (
          <Badge color={openRouterHealth.ok ? "green" : "red"} size="1">
            {openRouterHealth.ok ? "Key OK" : "Key Error"}
          </Badge>
        )}
        {parsedSchema.description && (
          <Text size="1" color="gray" style={{ flex: 1 }}>
            {parsedSchema.description.trim().split("\n")[0]}
          </Text>
        )}
      </Flex>

      {claudeUsage?.data && !claudeUsage.error && (
        <Flex direction="column" gap="2">
          <Text size="2" weight="medium">
            Usage
          </Text>
          {claudeUsage.data.five_hour && (
            <ClaudeWindowRow
              label="Session (5 hour)"
              w={claudeUsage.data.five_hour}
            />
          )}
          {claudeUsage.data.seven_day && (
            <ClaudeWindowRow label="Weekly" w={claudeUsage.data.seven_day} />
          )}
          {claudeUsage.data.extra_usage && (
            <Flex direction="column" gap="1">
              <Flex justify="between">
                <Text size="1" color="gray">
                  Extra usage
                </Text>
                <Text size="1" color="gray">
                  {claudeUsage.data.extra_usage.is_enabled
                    ? "enabled"
                    : "disabled"}
                  {" · "}${claudeUsage.data.extra_usage.used_credits.toFixed(2)}{" "}
                  spent
                  {typeof claudeUsage.data.extra_usage.monthly_limit ===
                  "number"
                    ? ` / $${claudeUsage.data.extra_usage.monthly_limit.toFixed(
                        0,
                      )} limit`
                    : " / unlimited"}
                </Text>
              </Flex>
              {typeof claudeUsage.data.extra_usage.utilization === "number" && (
                <UsageBar
                  pct={Math.max(
                    0,
                    Math.min(claudeUsage.data.extra_usage.utilization, 100),
                  )}
                />
              )}
            </Flex>
          )}
        </Flex>
      )}
      {(claudeUsage?.error != null || claudeUsageError) && (
        <Text size="1" color="gray">
          Usage: {claudeUsage?.error ?? "Failed to load"}
        </Text>
      )}

      {codexUsage?.data && !codexUsage.error && (
        <Flex direction="column" gap="2">
          <Flex align="center" gap="2">
            <Text size="2" weight="medium">
              Usage
            </Text>
            {codexUsage.data.plan_type && (
              <Badge color="blue" size="1">
                {codexUsage.data.plan_type}
              </Badge>
            )}
          </Flex>
          {codexUsage.data.rate_limit && (
            <>
              {codexUsage.data.rate_limit.primary_window && (
                <CodexWindowRow
                  label="Session (5 hour)"
                  w={codexUsage.data.rate_limit.primary_window}
                  limitReached={codexUsage.data.rate_limit.limit_reached}
                />
              )}
              {codexUsage.data.rate_limit.secondary_window && (
                <CodexWindowRow
                  label="Weekly"
                  w={codexUsage.data.rate_limit.secondary_window}
                />
              )}
            </>
          )}
          {codexUsage.data.code_review_rate_limit?.primary_window && (
            <CodexWindowRow
              label="Code review (weekly)"
              w={codexUsage.data.code_review_rate_limit.primary_window}
              limitReached={
                codexUsage.data.code_review_rate_limit.limit_reached
              }
            />
          )}
          {codexUsage.data.credits && (
            <Text size="1" color="gray">
              Credits:{" "}
              {codexUsage.data.credits.unlimited
                ? "unlimited"
                : codexUsage.data.credits.has_credits
                  ? `${codexUsage.data.credits.balance} remaining`
                  : "none"}
            </Text>
          )}
        </Flex>
      )}
      {(codexUsage?.error != null || codexUsageError) && (
        <Text size="1" color="gray">
          Usage: {codexUsage?.error ?? "Failed to load"}
        </Text>
      )}

      <Flex direction="column" width="100%" gap="3">
        {hasOAuth && (
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
            {importantFields.length > 0 && <Separator size="4" />}
          </>
        )}

        <Flex direction="column" gap="3">
          {importantFields.map((field) => (
            <SchemaField
              key={field.key}
              field={field}
              value={formValues[field.key]}
              disabled={isReadonly}
              onSave={handleFieldSave}
            />
          ))}
        </Flex>

        {extraFields.length > 0 && (
          <>
            <Flex align="center" justify="center">
              <Button
                className={styles.extraButton}
                variant="ghost"
                color="gray"
                size="1"
                onClick={() => setAreShowingExtraFields((prev) => !prev)}
              >
                {areShowingExtraFields ? "Hide" : "Show"} advanced fields
              </Button>
            </Flex>

            {areShowingExtraFields && (
              <Flex direction="column" gap="3">
                {extraFields.map((field) => (
                  <SchemaField
                    key={field.key}
                    field={field}
                    value={formValues[field.key]}
                    disabled={isReadonly}
                    onSave={handleFieldSave}
                  />
                ))}
              </Flex>
            )}
          </>
        )}
      </Flex>

      {hasCredentials && <ProviderModelsList provider={currentProvider} />}

      {hasCredentials && baseProvider === "custom" && detailedProvider && (
        <RoleSeparatedModelConfig provider={detailedProvider} />
      )}

      {hasCredentials && <ProviderDefaultModelsSetup />}
    </Flex>
  );
};
