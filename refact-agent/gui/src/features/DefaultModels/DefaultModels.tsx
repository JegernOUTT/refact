import React, { useState, useCallback, useEffect, useMemo } from "react";
import { Flex, Button, Text, Card, Heading, Callout } from "@radix-ui/themes";
import { ArrowLeftIcon, ExclamationTriangleIcon } from "@radix-ui/react-icons";

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

import type { Config } from "../Config/configSlice";

import styles from "./DefaultModels.module.css";

type DefaultModelsProps = {
  backFromDefaultModels: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
};

type ModelTypeKey = "chat" | "chat_light" | "chat_thinking";

const MODEL_TYPE_LABELS: Record<
  ModelTypeKey,
  { title: string; description: string }
> = {
  chat: {
    title: "Default Chat Model",
    description: "The primary model used for chat conversations",
  },
  chat_light: {
    title: "Light Chat Model",
    description: "Fast, lightweight model for quick responses and subagents",
  },
  chat_thinking: {
    title: "Thinking Model",
    description: "Reasoning-focused model for complex analysis tasks",
  },
};

const ModelTypeSection: React.FC<{
  typeKey: ModelTypeKey;
  config: ModelTypeDefaults;
  capsDefault: string;
  onChange: (key: ModelTypeKey, config: ModelTypeDefaults) => void;
}> = ({ typeKey, config, capsDefault, onChange }) => {
  const { title, description } = MODEL_TYPE_LABELS[typeKey];

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
          />
        </Flex>

        <ModelSamplingParams
          model={effectiveModel}
          values={config}
          onChange={handleSamplingChange}
          size="2"
        />
      </Flex>
    </Card>
  );
};

export const DefaultModels: React.FC<DefaultModelsProps> = ({
  backFromDefaultModels,
  host,
  tabbed,
}) => {
  const {
    data: defaults,
    isLoading,
    isSuccess,
    isError,
    refetch,
  } = useGetDefaultsQuery(undefined);
  const { data: capsData } = useGetCapsQuery(undefined);
  const [updateDefaults, { isLoading: isSaving }] = useUpdateDefaultsMutation();

  const capsDefaults = useMemo(
    () => ({
      chat: capsData?.chat_default_model ?? "",
      chat_light: capsData?.chat_light_model ?? "",
      chat_thinking: capsData?.chat_thinking_model ?? "",
    }),
    [capsData],
  );

  const [localDefaults, setLocalDefaults] = useState<ProviderDefaults>({
    chat: {},
    chat_light: {},
    chat_thinking: {},
  });

  const [hasChanges, setHasChanges] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (defaults) {
      setLocalDefaults(defaults);
      setHasChanges(false);
    }
  }, [defaults]);

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

  const handleSave = useCallback(async () => {
    try {
      await updateDefaults(localDefaults).unwrap();
      setHasChanges(false);
      setSaveError(null);
    } catch {
      setSaveError("Failed to save defaults. Please try again.");
    }
  }, [localDefaults, updateDefaults]);

  if (isLoading) {
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
                config={localDefaults[key]}
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
