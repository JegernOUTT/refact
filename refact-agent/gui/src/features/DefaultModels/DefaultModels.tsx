import React, { useState, useCallback, useEffect, useMemo } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  Bot,
  Boxes,
  Brain,
  Code,
  Info,
  MessageCircle,
  MessagesSquare,
  Rabbit,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";

import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
import { ModelSelector } from "../../components/Chat/ModelSelector";
import {
  ModelSamplingParams,
  type SamplingValues,
} from "../../components/ModelSamplingParams";
import { Button, Icon, SettingItem, Tabs } from "../../components/ui";

import {
  useGetDefaultsQuery,
  useUpdateDefaultsMutation,
  type ModelTypeDefaults,
  type ProviderDefaults,
} from "../../services/refact/providers";
import { useGetCapsQuery } from "../../services/refact/caps";
import { useGetDraftQuery } from "../../services/refact/buddy";

import type { Config } from "../Config/configSlice";
import { BuddyDraftPreview } from "../Buddy/BuddyDraftPreview";
import { SettingsGroup, SettingsSection } from "../Settings/SettingsSection";

import styles from "./DefaultModels.module.css";

type DefaultModelsProps = {
  backFromDefaultModels: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  draftId?: string;
  embedded?: boolean;
};

type ModelTypeKey =
  | "chat"
  | "chat_model_2"
  | "task_planner_agent_model"
  | "chat_light"
  | "chat_thinking"
  | "chat_buddy"
  | "completion_model"
  | "embedding_model";

type ModelTypeCapability = "chat" | "completion" | "embedding";

type ChatModelTypeKey =
  | "chat"
  | "chat_model_2"
  | "task_planner_agent_model"
  | "chat_light"
  | "chat_thinking"
  | "chat_buddy";

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
  completion_model: {
    title: "Completion Model",
    shortLabel: "Completion",
    description: "Model used for code completion and fill-in-middle requests.",
    icon: Code,
  },
  embedding_model: {
    title: "Embedding Model",
    shortLabel: "Embedding",
    description: "Model used for semantic search and VecDB embeddings.",
    icon: Boxes,
  },
};

const MODEL_TYPE_KEYS = Object.keys(MODEL_TYPE_LABELS) as ModelTypeKey[];

function getModelCapability(typeKey: ModelTypeKey): ModelTypeCapability {
  if (typeKey === "completion_model") return "completion";
  if (typeKey === "embedding_model") return "embedding";
  return "chat";
}

function getDefaultConfig(
  defaults: ProviderDefaults,
  key: ModelTypeKey,
): ModelTypeDefaults {
  const value = defaults[key];
  if (typeof value === "string") return { model: value };
  return (value as ModelTypeDefaults | undefined) ?? {};
}

function updateDefaultConfig(
  defaults: ProviderDefaults,
  key: ModelTypeKey,
  config: ModelTypeDefaults,
): ProviderDefaults {
  if (key === "completion_model" || key === "embedding_model") {
    return { ...defaults, [key]: config.model };
  }
  return { ...defaults, [key]: config };
}

const ModelTypeSection: React.FC<{
  typeKey: ModelTypeKey;
  config: ModelTypeDefaults;
  capsDefault: string;
  onChange: (key: ModelTypeKey, config: ModelTypeDefaults) => void;
}> = ({ typeKey, config, capsDefault, onChange }) => {
  const { title, description } = MODEL_TYPE_LABELS[typeKey];
  const capability = getModelCapability(typeKey);

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

  return (
    <div className={`${styles.content} rf-enter`}>
      <div className={styles.roleHeader}>
        <Icon icon={MODEL_TYPE_LABELS[typeKey].icon} size="lg" tone="accent" />
        <h2 className={styles.roleTitle}>{title}</h2>
        <p className={styles.description}>{description}</p>
      </div>

      <SettingsGroup title="Model Slot">
        <SettingItem
          className="rf-enter"
          title="Model"
          description="Choose the model override for this slot, or leave it empty to use the server default."
          control={
            <div className={styles.selectorWrap}>
              <ModelSelector
                value={config.model}
                onValueChange={handleModelChange}
                defaultValue={capsDefault}
                showLabel={false}
                compact={false}
                allowUnset
                unsetLabel="None"
                capability={capability}
              />
            </div>
          }
        />
      </SettingsGroup>

      {effectiveModel && capability !== "embedding" ? (
        <SettingsGroup title="Sampling">
          <SettingItem
            className="rf-enter"
            title="Sampling"
            description="Tune output length and reasoning behavior for this model slot."
            layout="stack"
          >
            <div className={styles.samplingWrap}>
              <ModelSamplingParams
                model={effectiveModel}
                values={config}
                onChange={handleSamplingChange}
                capability={capability}
              />
            </div>
          </SettingItem>
        </SettingsGroup>
      ) : !effectiveModel ? (
        <div className={`${styles.notice} rf-enter`}>
          <Icon icon={Info} size="sm" tone="muted" />
          <span>
            No model selected. Features that require this model type will ask
            you to configure it.
          </span>
        </div>
      ) : null}
    </div>
  );
};

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
  const {
    data: draft,
    isLoading: draftLoading,
    error: draftError,
  } = useGetDraftQuery(draftId ?? skipToken);
  const [updateDefaults, { isLoading: isSaving }] = useUpdateDefaultsMutation();

  const capsDefaults = useMemo(
    () => ({
      chat: capsData?.chat_default_model ?? "",
      chat_model_2: capsData?.chat_model_2 ?? "",
      task_planner_agent_model: capsData?.task_planner_agent_model ?? "",
      chat_light: capsData?.chat_light_model ?? "",
      chat_thinking: capsData?.chat_thinking_model ?? "",
      chat_buddy: capsData?.chat_buddy_model ?? "",
      completion_model: capsData?.completion_default_model ?? "",
      embedding_model:
        capsData?.embedding_model?.id ?? capsData?.embedding_model?.name ?? "",
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

  useEffect(() => {
    if (draftError) {
      setDraftExpired(true);
    }
  }, [draftError]);

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
          ] as ChatModelTypeKey[]) {
            const patchValue = patch[key];
            if (patchValue) {
              merged[key] = { ...getDefaultConfig(base, key), ...patchValue };
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
      setLocalDefaults((prev) => updateDefaultConfig(prev, key, config));
      setHasChanges(true);
      setSaveError(null);
    },
    [],
  );

  const handleSave = useCallback(async () => {
    try {
      const payload = draftId
        ? { ...localDefaults, draft_id: draftId }
        : localDefaults;
      await updateDefaults(payload).unwrap();
      void refetchCaps();
      setHasChanges(false);
      setSaveError(null);
    } catch {
      setSaveError("Failed to save defaults. Please try again.");
    }
  }, [draftId, localDefaults, refetchCaps, updateDefaults]);

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

  const saveAction = (
    <Button
      onClick={() => void handleSave()}
      disabled={!hasChanges || isSaving}
      loading={isSaving}
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

  const roleTabContents = MODEL_TYPE_KEYS.map((key) => (
    <Tabs.Content key={key} value={key} className={styles.roleTabContent}>
      <ModelTypeSection
        typeKey={key}
        config={getDefaultConfig(localDefaults, key)}
        capsDefault={capsDefaults[key]}
        onChange={handleModelTypeChange}
      />
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
            {roleTabContents}
          </SettingsSection>
        </Tabs>
      </div>
    </PageWrapper>
  );
};
