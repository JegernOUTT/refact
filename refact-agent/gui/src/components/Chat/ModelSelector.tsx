import React, { useCallback, useMemo } from "react";

import styles from "./ModelSelector.module.css";

import { useGetCapsQuery } from "../../hooks";
import { CapabilityIcons } from "../../features/Providers/ProviderForm/ProviderModelsList/components";
import {
  formatContextWindow,
  formatPricing,
} from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import {
  enrichAndGroupModels,
  type ModelSelectorCapability,
} from "../../utils/enrichModels";
import { isLegacyRefactModel } from "../../utils/modelProviders";
import {
  ModelSelector as KitModelSelector,
  type ModelOption,
  type ModelSelectorBadge,
  type ModelSelectorGroup,
} from "../ui";

export type ModelSelectorProps = {
  disabled?: boolean;
  value: string | undefined;
  onValueChange: (model: string) => void;
  label?: string;
  showLabel?: boolean;
  compact?: boolean;
  defaultValue?: string;
  allowUnset?: boolean;
  unsetLabel?: string;
  capability?: ModelSelectorCapability;
};

function modelBadges(
  model: ReturnType<typeof enrichAndGroupModels>[number]["models"][number],
) {
  const badges: ModelSelectorBadge[] = [];
  if (model.isDefault) badges.push("default");
  if (model.isThinking) badges.push("reasoning");
  if (model.isLight) badges.push("light");
  if (model.isBuddy) badges.push("buddy");
  if (model.isTaskPlannerAgent) badges.push("task-agent");
  if (model.isChat2) badges.push("chat2");
  return badges;
}

function pricingOption(
  model: ReturnType<typeof enrichAndGroupModels>[number]["models"][number],
) {
  if (!model.pricing) return undefined;
  const [prompt, output] = formatPricing(model.pricing, true).split("/");
  return { prompt, output };
}

export const ModelSelector: React.FC<ModelSelectorProps> = ({
  disabled,
  value,
  onValueChange,
  label = "model:",
  showLabel = true,
  compact = true,
  defaultValue,
  allowUnset = false,
  unsetLabel = "None",
  capability = "chat",
}) => {
  const { data: caps } = useGetCapsQuery(undefined);

  const usableModels = useMemo(() => {
    if (capability === "completion") {
      return Object.keys(caps?.completion_models ?? {}).map((model) => ({
        value: model,
        disabled: false,
        textValue: model,
      }));
    }

    if (capability === "embedding") {
      const embeddingModel = caps?.embedding_model;
      if (!embeddingModel) return [];
      return [
        {
          value: embeddingModel.id,
          disabled: false,
          textValue: embeddingModel.id,
        },
      ];
    }

    return Object.keys(caps?.chat_models ?? {})
      .filter((model) => !isLegacyRefactModel(model))
      .map((model) => ({
        value: model,
        disabled: false,
        textValue: model,
      }));
  }, [
    capability,
    caps?.chat_models,
    caps?.completion_models,
    caps?.embedding_model,
  ]);

  const groupedModels = useMemo(
    () => enrichAndGroupModels(usableModels, caps, capability),
    [usableModels, caps, capability],
  );

  const defaultModel =
    defaultValue ??
    (capability === "completion"
      ? caps?.completion_default_model
      : capability === "embedding"
        ? caps?.embedding_model?.id
        : caps?.chat_default_model) ??
    "";
  const effectiveValue = value ?? defaultModel;
  const hasEffectiveValueInList = groupedModels.some((group) =>
    group.models.some((model) => model.value === effectiveValue),
  );

  const groups = useMemo<ModelSelectorGroup[]>(() => {
    return groupedModels.map((group) => ({
      id: group.provider,
      label: group.displayName,
    }));
  }, [groupedModels]);

  const models = useMemo<ModelOption[]>(() => {
    const mappedModels = groupedModels.flatMap((group) =>
      group.models.map((model) => ({
        value: model.value,
        displayName: model.value,
        group: group.provider,
        disabled: model.disabled,
        pricing: pricingOption(model),
        contextWindow: model.nCtx ? formatContextWindow(model.nCtx) : undefined,
        badges: modelBadges(model),
        capabilities: model.capabilities ? (
          <CapabilityIcons capabilities={model.capabilities} />
        ) : undefined,
      })),
    );

    if (effectiveValue && !hasEffectiveValueInList) {
      return [
        {
          value: effectiveValue,
          displayName: effectiveValue,
          disabled: true,
        },
        ...mappedModels,
      ];
    }

    return mappedModels;
  }, [effectiveValue, groupedModels, hasEffectiveValueInList]);

  const handleSelect = useCallback(
    (nextValue: string) => {
      onValueChange(nextValue === "" ? "" : nextValue);
    },
    [onValueChange],
  );

  const triggerSize = compact ? "sm" : "md";

  if (!caps && models.length === 0) {
    return (
      <span className={styles.fallbackText}>
        {showLabel ? `${label} ` : ""}
        {allowUnset && !effectiveValue
          ? unsetLabel
          : effectiveValue || "No models"}
      </span>
    );
  }

  return (
    <div className={compact ? styles.compact : styles.stack}>
      {showLabel ? <span className={styles.label}>{label}</span> : null}
      <KitModelSelector
        allowUnset={allowUnset}
        disabled={disabled}
        groups={groups}
        models={models}
        unsetLabel={unsetLabel}
        triggerSize={triggerSize}
        value={allowUnset && !effectiveValue ? null : effectiveValue || null}
        onSelect={handleSelect}
      />
    </div>
  );
};
