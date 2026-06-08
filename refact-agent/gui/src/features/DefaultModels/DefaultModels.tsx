import React, { useState, useCallback, useEffect, useMemo } from "react";
import { Flex, Button, Text, Card, Heading, Callout } from "@radix-ui/themes";
import {
  ArrowLeftIcon,
  ExclamationTriangleIcon,
  InfoCircledIcon,
} from "@radix-ui/react-icons";
import { skipToken } from "@reduxjs/toolkit/query";

import { ScrollArea } from "../../components/ScrollArea";
import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
import { ModelSelector } from "../../components/Chat/ModelSelector";
import {
  ModelSamplingParams,
  type SamplingValues,
} from "../../components/ModelSamplingParams";

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

import styles from "./DefaultModels.module.css";

type DefaultModelsProps = {
  backFromDefaultModels: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  draftId?: string;
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

const MODEL_TYPE_LABELS: Record<
  ModelTypeKey,
  { title: string; description: string }
> = {
  chat: {
    title: "Default Chat Model",
    description: "The primary model used for chat conversations",
  },
  chat_model_2: {
    title: "Chat Model 2",
    description: "Secondary chat model slot for future chat workflows",
  },
  task_planner_agent_model: {
    title: "Task Planner Agent Model",
    description: "Model used by task management when spawning task agents",
  },
  chat_light: {
    title: "Light Chat Model",
    description: "Fast, lightweight model for quick responses and subagents",
  },
  chat_thinking: {
    title: "Thinking Model",
    description: "Reasoning-focused model for complex analysis tasks",
  },
  chat_buddy: {
    title: "Companion Model",
    description:
      "Model used by your companion for background tasks and suggestions",
  },
  completion_model: {
    title: "Completion Model",
    description: "Model used for code completion and fill-in-middle requests",
  },
  embedding_model: {
    title: "Embedding Model",
    description: "Model used for semantic search and VecDB embeddings",
  },
};

function getModelCapability(typeKey: ModelTypeKey) {
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
  return value ?? {};
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
    <Card className={styles.modelTypeCard}>
      <Flex direction="column" gap="4">
        <Flex direction="column" gap="1">
          <Heading size="3">{title}</Heading>
          <Text size="2" color="gray">
            {description}
          </Text>
        </Flex>

        <Flex direction="column" gap="2">
          <Text size="2" weight="medium">
            Model
          </Text>
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
        </Flex>

        {effectiveModel ? (
          <ModelSamplingParams
            model={effectiveModel}
            values={config}
            onChange={handleSamplingChange}
            size="2"
            capability={capability}
          />
        ) : (
          <Callout.Root color="gray">
            <Callout.Icon>
              <InfoCircledIcon />
            </Callout.Icon>
            <Callout.Text>
              No model selected. Features that require this model type will ask
              you to configure it.
            </Callout.Text>
          </Callout.Root>
        )}
      </Flex>
    </Card>
  );
};

export const DefaultModels: React.FC<DefaultModelsProps> = ({
  backFromDefaultModels,
  host,
  tabbed,
  draftId,
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

  const [localDefaults, setLocalDefaults] = useState<ProviderDefaults>({
    chat: {},
    chat_model_2: {},
    task_planner_agent_model: {},
    chat_light: {},
    chat_thinking: {},
    chat_buddy: {},
    completion_model: undefined,
    embedding_model: undefined,
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
        ...defaults,
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
          ] as const) {
            const patchValue = patch[key];
            if (patchValue) {
              merged[key] = {
                ...getDefaultConfig(base, key),
                ...patchValue,
              };
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
      void refetch();
      void refetchCaps();
      setHasChanges(false);
      setSaveError(null);
    } catch {
      setSaveError("Failed to save defaults. Please try again.");
    }
  }, [draftId, localDefaults, refetch, refetchCaps, updateDefaults]);

  if (isLoading || draftLoading) {
    return <Spinner spinning />;
  }

  if (isError || !isSuccess) {
    return (
      <PageWrapper host={host}>
        <Flex direction="column" gap="4" p="4" align="center" justify="center">
          <Callout.Root color="red">
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>
              Failed to load default models configuration.
            </Callout.Text>
          </Callout.Root>
          <Button onClick={() => void refetch()}>Retry</Button>
          <Button variant="outline" onClick={backFromDefaultModels}>
            Back
          </Button>
        </Flex>
      </PageWrapper>
    );
  }

  return (
    <PageWrapper
      host={host}
      style={{
        padding: 0,
        marginTop: 0,
      }}
    >
      <Flex direction="column" gap="4" p="4" style={{ height: "100%" }}>
        <Flex justify="between" align="center">
          {host === "vscode" && !tabbed ? (
            <Button variant="surface" onClick={backFromDefaultModels}>
              <ArrowLeftIcon width="16" height="16" />
              Back
            </Button>
          ) : (
            <Button variant="outline" onClick={backFromDefaultModels}>
              Back
            </Button>
          )}

          <Button
            onClick={() => void handleSave()}
            disabled={!hasChanges || isSaving}
            variant="solid"
          >
            {isSaving ? "Saving..." : "Save Changes"}
          </Button>
        </Flex>

        {draftExpired && (
          <Callout.Root color="orange">
            <Callout.Icon>
              <InfoCircledIcon />
            </Callout.Icon>
            <Callout.Text>Draft expired</Callout.Text>
          </Callout.Root>
        )}

        {draft && <BuddyDraftPreview draft={draft} />}

        {saveError && (
          <Callout.Root color="red">
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>{saveError}</Callout.Text>
          </Callout.Root>
        )}

        <Flex direction="column" gap="2">
          <Heading size="5">Default Models</Heading>
          <Text size="2" color="gray">
            Configure which models to use by default for different purposes.
            These settings apply globally across all modes.
          </Text>
        </Flex>

        <ScrollArea scrollbars="vertical" fullHeight>
          <Flex direction="column" gap="4" pb="4">
            {(Object.keys(MODEL_TYPE_LABELS) as ModelTypeKey[]).map((key) => (
              <ModelTypeSection
                key={key}
                typeKey={key}
                config={getDefaultConfig(localDefaults, key)}
                capsDefault={capsDefaults[key]}
                onChange={handleModelTypeChange}
              />
            ))}
          </Flex>
        </ScrollArea>
      </Flex>
    </PageWrapper>
  );
};
