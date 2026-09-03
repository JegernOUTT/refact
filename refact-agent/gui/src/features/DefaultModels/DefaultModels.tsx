import React, { useState, useCallback, useEffect, useMemo } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  Bot,
  Brain,
  Info,
  MessageCircle,
  MessagesSquare,
  Rabbit,
  RotateCcw,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";

import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
import { ModelSelector } from "../../components/Chat/ModelSelector";
import type { SamplingValues } from "../../components/ModelSamplingParams";
import {
  Badge,
  Button,
  FieldSlider,
  FieldSwitch,
  Icon,
  IconButton,
  SegmentedControl,
  SettingItem,
  Tabs,
} from "../../components/ui";

import {
  useGetDefaultsQuery,
  useGetProjectDefaultsQuery,
  useUpdateDefaultsMutation,
  useUpdateProjectDefaultsMutation,
  type ModelTypeDefaults,
  type ProjectModelDefaults,
  type ProjectModelSlotKey,
  type ProviderDefaults,
} from "../../services/refact/providers";
import { useGetCapsQuery } from "../../services/refact/caps";
import { useGetDraftQuery } from "../../services/refact/buddy";

import type { Config } from "../Config/configSlice";
import { BuddyDraftPreview } from "../Buddy/BuddyDraftPreview";
import { ReasoningIcon } from "../Providers/ProviderForm/ProviderModelsList/components/CapabilityIcons";
import { SettingsGroup, SettingsSection } from "../Settings/SettingsSection";

import styles from "./DefaultModels.module.css";

type DefaultModelsProps = {
  backFromDefaultModels: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  draftId?: string;
  embedded?: boolean;
};

type ModelTypeKey = ProjectModelSlotKey;

type ModelDefaultsScope = "global" | "project";

const MODEL_TYPE_LABELS: Record<
  ModelTypeKey,
  { title: string; shortLabel: string; description: string; icon: LucideIcon }
> = {
  chat: {
    title: "Default Chat Model",
    shortLabel: "Chat",
    description: "The primary model used for chat conversations.",
    icon: MessageCircle,
  },
  chat_model_2: {
    title: "Chat Model 2",
    shortLabel: "Chat 2",
    description: "Secondary chat model slot for future chat workflows.",
    icon: MessagesSquare,
  },
  task_planner_agent_model: {
    title: "Task Planner Agent Model",
    shortLabel: "Planner",
    description: "Model used by task management when spawning task agents.",
    icon: Bot,
  },
  chat_light: {
    title: "Light Chat Model",
    shortLabel: "Light",
    description: "Fast, lightweight model for quick responses and subagents.",
    icon: Zap,
  },
  chat_thinking: {
    title: "Thinking Model",
    shortLabel: "Thinking",
    description: "Reasoning-focused model for complex analysis tasks.",
    icon: Brain,
  },
  chat_buddy: {
    title: "Companion Model",
    shortLabel: "Companion",
    description:
      "Model used by your companion for background tasks and suggestions.",
    icon: Rabbit,
  },
};

const MODEL_TYPE_KEYS = Object.keys(MODEL_TYPE_LABELS) as ModelTypeKey[];

const SERVER_DEFAULT_LABEL = "Server default";

function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(tokens % 1000000 === 0 ? 0 : 1)}M`;
  }
  return `${Math.round(tokens / 1000)}K`;
}

const ModelTypeSection: React.FC<{
  typeKey: ModelTypeKey;
  config: ModelTypeDefaults;
  capsDefault: string;
  onChange: (key: ModelTypeKey, config: ModelTypeDefaults) => void;
  allowUnset?: boolean;
  leading?: React.ReactNode;
}> = ({
  typeKey,
  config,
  capsDefault,
  onChange,
  allowUnset = true,
  leading,
}) => {
  const { title, description } = MODEL_TYPE_LABELS[typeKey];
  const { data: capsData } = useGetCapsQuery(undefined);

  const handleModelChange = useCallback(
    (model: string) => {
      onChange(typeKey, { ...config, model });
    },
    [typeKey, config, onChange],
  );

  const handleSamplingChange = useCallback(
    <K extends keyof SamplingValues>(field: K, value: SamplingValues[K]) => {
      onChange(typeKey, { ...config, [field]: value });
    },
    [typeKey, config, onChange],
  );

  const effectiveModel = config.model ?? capsDefault;
  const chatModels: Record<string, unknown> | undefined = capsData?.chat_models;
  const modelDetail = effectiveModel
    ? (chatModels?.[effectiveModel] as
        | {
            default_max_tokens?: number | null;
            max_output_tokens?: number | null;
            reasoning_effort_options?: string[] | null;
            supports_thinking_budget?: boolean;
            supports_adaptive_thinking_budget?: boolean;
          }
        | undefined)
    : undefined;
  const defaultMaxTokens = modelDetail?.default_max_tokens ?? 4096;
  const maxOutputTokens = modelDetail?.max_output_tokens ?? 16384;
  const reasoningEffortOptions = modelDetail?.reasoning_effort_options;
  const supportsThinkingBudget = modelDetail?.supports_thinking_budget ?? false;
  const supportsReasoning =
    (reasoningEffortOptions != null && reasoningEffortOptions.length > 0) ||
    supportsThinkingBudget;

  return (
    <div className={`${styles.content} rf-enter`}>
      <SettingsGroup title={title} description={description}>
        {leading}
        <SettingItem
          className="rf-enter"
          title="Model"
          description={
            allowUnset
              ? "Choose the model override for this slot, or leave it empty to use the server default."
              : "Choose the model used for this slot in the current project."
          }
          control={
            <div className={styles.selectorWrap} title={effectiveModel}>
              <ModelSelector
                value={config.model}
                onValueChange={handleModelChange}
                defaultValue={capsDefault}
                showLabel={false}
                compact={false}
                allowUnset={allowUnset}
                unsetLabel="None"
              />
            </div>
          }
        />

        {effectiveModel ? (
          <>
            {supportsReasoning ? (
              <SettingItem
                className="rf-enter"
                title={
                  <span className={styles.reasoningLabel}>
                    <ReasoningIcon />
                    Reasoning
                  </span>
                }
                description="Use additional reasoning controls for this model."
                control={
                  <FieldSwitch
                    aria-label="Reasoning"
                    checked={config.boost_reasoning ?? false}
                    onChange={(checked) => {
                      handleSamplingChange(
                        "boost_reasoning",
                        checked || undefined,
                      );
                      if (!checked) {
                        handleSamplingChange("reasoning_effort", undefined);
                        handleSamplingChange("thinking_budget", undefined);
                      }
                    }}
                  />
                }
              />
            ) : null}

            {supportsReasoning && config.boost_reasoning ? (
              <>
                {reasoningEffortOptions != null &&
                reasoningEffortOptions.length > 0 ? (
                  <SettingItem
                    className="rf-enter"
                    title="Effort"
                    description="Choose how much reasoning the model should apply."
                    control={
                      <SegmentedControl
                        className={styles.segmented}
                        size="sm"
                        value={config.reasoning_effort ?? "medium"}
                        onValueChange={(level) =>
                          handleSamplingChange("reasoning_effort", level)
                        }
                        options={reasoningEffortOptions.map((level) => ({
                          value: level,
                          label: level,
                        }))}
                      />
                    }
                  />
                ) : null}

                {supportsThinkingBudget ? (
                  <SettingItem
                    className="rf-enter"
                    title="Thinking tokens"
                    description="Set the token budget available for reasoning."
                    layout="stack"
                    control={
                      <FieldSlider
                        label={`${formatTokens(1024)} – ${formatTokens(32768)}`}
                        valueLabel={config.thinking_budget ?? 16384}
                        min={1024}
                        max={32768}
                        step={1024}
                        value={[config.thinking_budget ?? 16384]}
                        onChange={(value) =>
                          handleSamplingChange("thinking_budget", value[0])
                        }
                        aria-label="Thinking tokens"
                      />
                    }
                  />
                ) : null}
              </>
            ) : null}

            <SettingItem
              className="rf-enter"
              title="Max tokens"
              description="Set the maximum length of the model response."
              layout="stack"
              control={
                <div className={styles.valueControl}>
                  <FieldSlider
                    label={`${formatTokens(1024)} – ${formatTokens(
                      maxOutputTokens,
                    )}`}
                    valueLabel={
                      config.max_new_tokens ?? `${defaultMaxTokens} (default)`
                    }
                    min={1024}
                    max={maxOutputTokens}
                    step={1024}
                    value={[config.max_new_tokens ?? defaultMaxTokens]}
                    onChange={(value) =>
                      handleSamplingChange("max_new_tokens", value[0])
                    }
                    aria-label="Max tokens"
                  />
                  {config.max_new_tokens != null ? (
                    <IconButton
                      icon={RotateCcw}
                      size="sm"
                      variant="plain"
                      onClick={() =>
                        handleSamplingChange("max_new_tokens", undefined)
                      }
                      aria-label="Reset max tokens"
                    />
                  ) : null}
                </div>
              }
            />
          </>
        ) : (
          <div className={`${styles.notice} rf-enter`}>
            <Icon icon={Info} size="sm" tone="muted" />
            <span>
              No model selected. Features that require this model type will ask
              you to configure it.
            </span>
          </div>
        )}
      </SettingsGroup>
    </div>
  );
};

function describeInheritedSlot(
  config: ModelTypeDefaults,
  capsDefault: string,
): string {
  const parts: string[] = [
    config.model ?? (capsDefault || SERVER_DEFAULT_LABEL),
  ];
  if (config.boost_reasoning) {
    parts.push(`Reasoning: ${config.reasoning_effort ?? "on"}`);
  } else {
    parts.push("Reasoning: off");
  }
  if (config.max_new_tokens != null) {
    parts.push(`Max tokens: ${config.max_new_tokens}`);
  }
  return parts.join(" · ");
}

export const DefaultModels: React.FC<DefaultModelsProps> = ({
  backFromDefaultModels,
  host,
  tabbed,
  draftId,
  embedded,
}) => {
  const {
    data: defaults,
    isLoading,
    isSuccess,
    isError,
    refetch,
  } = useGetDefaultsQuery(undefined);
  const { data: capsData, refetch: refetchCaps } = useGetCapsQuery(undefined);
  const { data: projectDefaults, isLoading: projectLoading } =
    useGetProjectDefaultsQuery(undefined);
  const {
    data: draft,
    isLoading: draftLoading,
    error: draftError,
  } = useGetDraftQuery(draftId ?? skipToken);
  const [updateDefaults, { isLoading: isSaving }] = useUpdateDefaultsMutation();
  const [updateProjectDefaults, { isLoading: isSavingProject }] =
    useUpdateProjectDefaultsMutation();

  const capsDefaults = useMemo(
    () => ({
      chat: capsData?.chat_default_model ?? "",
      chat_model_2: capsData?.chat_model_2 ?? "",
      task_planner_agent_model: capsData?.task_planner_agent_model ?? "",
      chat_light: capsData?.chat_light_model ?? "",
      chat_thinking: capsData?.chat_thinking_model ?? "",
      chat_buddy: capsData?.chat_buddy_model ?? "",
    }),
    [capsData],
  );

  const [activeSection, setActiveSection] = useState<ModelTypeKey>("chat");
  const [localDefaults, setLocalDefaults] = useState<ProviderDefaults>({
    chat: {},
    chat_model_2: {},
    task_planner_agent_model: {},
    chat_light: {},
    chat_thinking: {},
    chat_buddy: {},
  });

  const [hasChanges, setHasChanges] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [draftExpired, setDraftExpired] = useState(false);
  const [scope, setScope] = useState<ModelDefaultsScope>("global");
  const [localProjectDefaults, setLocalProjectDefaults] =
    useState<ProjectModelDefaults>({});
  const [hasProjectChanges, setHasProjectChanges] = useState(false);

  const projectAvailable = projectDefaults?.project_available ?? false;

  useEffect(() => {
    if (draftError) {
      setDraftExpired(true);
    }
  }, [draftError]);

  useEffect(() => {
    if (projectDefaults) {
      setLocalProjectDefaults(projectDefaults.defaults);
      setHasProjectChanges(false);
    }
  }, [projectDefaults]);

  useEffect(() => {
    if (!projectAvailable) {
      setScope("global");
    }
  }, [projectAvailable]);

  useEffect(() => {
    if (defaults) {
      const base: ProviderDefaults = {
        chat: defaults.chat,
        chat_model_2: defaults.chat_model_2,
        task_planner_agent_model: defaults.task_planner_agent_model,
        chat_light: defaults.chat_light,
        chat_thinking: defaults.chat_thinking,
        chat_buddy: defaults.chat_buddy ?? {},
        completion_model: defaults.completion_model,
        embedding_model: defaults.embedding_model,
      };
      let appliedDraft = false;
      if (draft && draft.kind === "defaults_model") {
        try {
          const patch = JSON.parse(draft.yaml_or_json) as Partial<
            Record<ModelTypeKey, Partial<ModelTypeDefaults>>
          >;
          const merged: ProviderDefaults = { ...base };
          for (const key of [
            "chat",
            "chat_light",
            "chat_thinking",
            "chat_buddy",
          ] as ModelTypeKey[]) {
            if (patch[key]) {
              merged[key] = { ...(base[key] ?? {}), ...patch[key] };
              appliedDraft = true;
            }
          }
          setLocalDefaults(merged);
        } catch {
          setLocalDefaults(base);
        }
      } else {
        setLocalDefaults(base);
      }
      setHasChanges(appliedDraft);
    }
  }, [defaults, draft]);

  const handleModelTypeChange = useCallback(
    (key: ModelTypeKey, config: ModelTypeDefaults) => {
      setLocalDefaults((prev) => ({
        ...prev,
        [key]: config,
      }));
      setHasChanges(true);
      setSaveError(null);
    },
    [],
  );

  const handleProjectTypeChange = useCallback(
    (key: ModelTypeKey, config: ModelTypeDefaults) => {
      setLocalProjectDefaults((prev) => ({
        ...prev,
        [key]: config,
      }));
      setHasProjectChanges(true);
      setSaveError(null);
    },
    [],
  );

  const handleProjectOverrideToggle = useCallback(
    (key: ModelTypeKey, enabled: boolean) => {
      setLocalProjectDefaults((prev) => {
        if (!enabled) {
          const next: ProjectModelDefaults = {};
          for (const slotKey of MODEL_TYPE_KEYS) {
            const slot = prev[slotKey];
            if (slotKey !== key && slot) {
              next[slotKey] = slot;
            }
          }
          return next;
        }
        const globalSlot = localDefaults[key] ?? {};
        return {
          ...prev,
          [key]: {
            ...globalSlot,
            model: globalSlot.model ?? (capsDefaults[key] || undefined),
          },
        };
      });
      setHasProjectChanges(true);
      setSaveError(null);
    },
    [capsDefaults, localDefaults],
  );

  const handleSave = useCallback(async () => {
    try {
      if (hasChanges) {
        const payload = draftId
          ? { ...localDefaults, draft_id: draftId }
          : localDefaults;
        await updateDefaults(payload).unwrap();
      }
      if (hasProjectChanges) {
        const payload: ProjectModelDefaults = {};
        for (const key of MODEL_TYPE_KEYS) {
          const slot = localProjectDefaults[key];
          if (slot?.model) {
            payload[key] = slot;
          }
        }
        await updateProjectDefaults(payload).unwrap();
      }
      void refetchCaps();
      setHasChanges(false);
      setHasProjectChanges(false);
      setSaveError(null);
    } catch {
      setSaveError("Failed to save defaults. Please try again.");
    }
  }, [
    draftId,
    hasChanges,
    hasProjectChanges,
    localDefaults,
    localProjectDefaults,
    refetchCaps,
    updateDefaults,
    updateProjectDefaults,
  ]);

  if (isLoading || draftLoading) {
    return <Spinner spinning />;
  }

  if (isError || !isSuccess) {
    const errorContent = (
      <div className={styles.page}>
        <div className={`${styles.notice} ${styles.noticeDanger}`}>
          <Icon icon={AlertTriangle} size="sm" tone="danger" />
          <span>Failed to load default models configuration.</span>
        </div>
        <div className={styles.actions}>
          <Button variant="soft" onClick={() => void refetch()}>
            Retry
          </Button>
          {!embedded && (
            <Button variant="ghost" onClick={backFromDefaultModels}>
              Back
            </Button>
          )}
        </div>
      </div>
    );
    if (embedded) return errorContent;
    return <PageWrapper host={host}>{errorContent}</PageWrapper>;
  }

  const activeKey = MODEL_TYPE_KEYS.includes(activeSection)
    ? activeSection
    : "chat";

  const isSavingAny = isSaving || isSavingProject;
  const isDirty = hasChanges || hasProjectChanges;

  const saveAction = (
    <Button
      onClick={() => void handleSave()}
      disabled={!isDirty || isSavingAny}
      loading={isSavingAny}
      variant="primary"
    >
      Save Changes
    </Button>
  );

  const headerActions = (
    <div className={styles.headerActions}>
      {!embedded && (
        <Button
          variant={host === "vscode" && !tabbed ? "soft" : "ghost"}
          leftIcon={ArrowLeft}
          onClick={backFromDefaultModels}
        >
          Back
        </Button>
      )}
      {saveAction}
    </div>
  );

  const roleTabsList = (
    <Tabs.List
      activeIndex={MODEL_TYPE_KEYS.indexOf(activeKey)}
      className={styles.roleTabsList}
      itemCount={MODEL_TYPE_KEYS.length}
    >
      {MODEL_TYPE_KEYS.map((key) => (
        <Tabs.Trigger key={key} value={key}>
          <span className={styles.roleTabLabel}>
            <Icon icon={MODEL_TYPE_LABELS[key].icon} size="sm" />
            <span>{MODEL_TYPE_LABELS[key].shortLabel}</span>
          </span>
        </Tabs.Trigger>
      ))}
    </Tabs.List>
  );

  const scopeGroup = (
    <SettingsGroup title="Configuration scope">
      <SettingItem
        className="rf-enter"
        title="Scope"
        description={
          scope === "global"
            ? "Global settings apply across projects."
            : "Project settings apply only to the currently open project."
        }
        control={
          <SegmentedControl
            name="model-defaults-scope"
            value={scope}
            options={[
              { value: "global", label: "Global" },
              {
                value: "project",
                label: "This project",
                disabled: projectLoading || !projectAvailable,
              },
            ]}
            onValueChange={(value) => {
              setScope(value as ModelDefaultsScope);
              setSaveError(null);
            }}
          />
        }
      />
    </SettingsGroup>
  );

  const renderProjectSlot = (key: ModelTypeKey) => {
    const projectSlot = localProjectDefaults[key];
    const globalSlot = localDefaults[key] ?? {};
    const overrideSwitch = (
      <SettingItem
        className="rf-enter"
        title="Override for this project"
        description="Use a project-specific model and parameters for this slot instead of the global default."
        control={
          <FieldSwitch
            aria-label="Override for this project"
            checked={projectSlot !== undefined}
            onChange={(checked) => handleProjectOverrideToggle(key, checked)}
          />
        }
      />
    );

    if (projectSlot === undefined) {
      return (
        <div className={`${styles.content} rf-enter`}>
          <SettingsGroup
            title={MODEL_TYPE_LABELS[key].title}
            description={MODEL_TYPE_LABELS[key].description}
          >
            {overrideSwitch}
            <SettingItem
              className="rf-enter"
              title="Inherited from global"
              description={describeInheritedSlot(globalSlot, capsDefaults[key])}
              control={<Badge tone="muted">Global</Badge>}
            />
          </SettingsGroup>
        </div>
      );
    }

    return (
      <>
        <ModelTypeSection
          typeKey={key}
          config={projectSlot}
          capsDefault={capsDefaults[key]}
          onChange={handleProjectTypeChange}
          allowUnset={false}
          leading={overrideSwitch}
        />
        {projectSlot.model ? null : (
          <div className={`${styles.notice} ${styles.noticeWarning} rf-enter`}>
            <Icon icon={AlertTriangle} size="sm" tone="warning" />
            <span>Pick a model to activate this override.</span>
          </div>
        )}
      </>
    );
  };

  const renderGlobalSlot = (key: ModelTypeKey) => {
    const projectModel = localProjectDefaults[key]?.model;

    return (
      <>
        {projectModel ? (
          <div className={`${styles.notice} ${styles.noticeAccent} rf-enter`}>
            <Icon icon={Info} size="sm" tone="accent" />
            <span>
              This project overrides this slot with {projectModel}. Switch to
              “This project” to change it.
            </span>
          </div>
        ) : null}
        <ModelTypeSection
          typeKey={key}
          config={localDefaults[key] ?? {}}
          capsDefault={capsDefaults[key]}
          onChange={handleModelTypeChange}
        />
      </>
    );
  };

  const roleTabContents = MODEL_TYPE_KEYS.map((key) => (
    <Tabs.Content key={key} value={key} className={styles.roleTabContent}>
      {scope === "project" ? renderProjectSlot(key) : renderGlobalSlot(key)}
    </Tabs.Content>
  ));

  const notices = (
    <>
      {draftExpired ? (
        <div className={`${styles.notice} ${styles.noticeAccent} rf-enter`}>
          <Icon icon={Info} size="sm" tone="accent" />
          <span>Draft expired</span>
        </div>
      ) : null}
      {draft ? <BuddyDraftPreview draft={draft} /> : null}
      {saveError ? (
        <div className={`${styles.notice} ${styles.noticeDanger} rf-enter`}>
          <Icon icon={AlertTriangle} size="sm" tone="danger" />
          <span>{saveError}</span>
        </div>
      ) : null}
    </>
  );

  if (embedded) {
    return (
      <div className={styles.page}>
        <Tabs
          value={activeKey}
          onValueChange={(v) => setActiveSection(v as ModelTypeKey)}
          className={styles.roleTabs}
        >
          <SettingsSection
            title="Models"
            description="Configure the default model slots used across chat, planning, quick responses, reasoning, and companion workflows."
            actions={saveAction}
            subNav={roleTabsList}
          >
            {notices}
            {scopeGroup}
            {roleTabContents}
          </SettingsSection>
        </Tabs>
      </div>
    );
  }

  return (
    <PageWrapper host={host}>
      <div className={styles.page}>
        <Tabs
          value={activeKey}
          onValueChange={(v) => setActiveSection(v as ModelTypeKey)}
          className={styles.roleTabs}
        >
          <SettingsSection
            title="Models"
            description="Configure the default model slots used across chat, planning, quick responses, reasoning, and companion workflows."
            actions={headerActions}
            subNav={roleTabsList}
          >
            {notices}
            {scopeGroup}
            {roleTabContents}
          </SettingsSection>
        </Tabs>
      </div>
    </PageWrapper>
  );
};
