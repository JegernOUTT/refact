import React, { useCallback, useMemo } from "react";
import { Select, Text, Flex } from "@radix-ui/themes";
import { useGetCapsQuery } from "../../hooks";
import { RichModelSelectItem } from "../Select/RichModelSelectItem";
import {
  enrichAndGroupModels,
  type ModelSelectorCapability,
} from "../../utils/enrichModels";
import { isLegacyRefactModel } from "../../utils/modelProviders";
import styles from "../Select/select.module.css";

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

const UNSET_MODEL_VALUE = "__refact_unset_model__";

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
  const firstModelValue = groupedModels[0]?.models[0]?.value ?? "";
  const selectValue =
    allowUnset && !effectiveValue
      ? UNSET_MODEL_VALUE
      : effectiveValue || firstModelValue;
  const currentModelName = effectiveValue;
  const triggerLabel =
    allowUnset && !effectiveValue ? unsetLabel : currentModelName;
  const hasEffectiveValueInList = groupedModels.some((group) =>
    group.models.some((model) => model.value === effectiveValue),
  );
  const showUnavailableValue = Boolean(
    effectiveValue && !hasEffectiveValueInList,
  );
  const handleValueChange = useCallback(
    (nextValue: string) => {
      onValueChange(nextValue === UNSET_MODEL_VALUE ? "" : nextValue);
    },
    [onValueChange],
  );

  const renderModelItem = (
    model: (typeof groupedModels)[number]["models"][number],
  ) => (
    <Select.Item
      key={model.value}
      value={model.value}
      disabled={model.disabled}
      textValue={model.value}
    >
      <span className={styles.trigger_only}>{model.value}</span>
      <span className={styles.dropdown_only}>
        <RichModelSelectItem
          displayName={model.value}
          pricing={model.pricing}
          nCtx={model.nCtx}
          capabilities={model.capabilities}
          metadata={model.metadata}
          isDefault={model.isDefault}
          isChat2={model.isChat2}
          isTaskPlannerAgent={model.isTaskPlannerAgent}
          isThinking={model.isThinking}
          isLight={model.isLight}
          isBuddy={model.isBuddy}
        />
      </span>
    </Select.Item>
  );

  const renderContent = () => (
    <Select.Content position="popper">
      {showUnavailableValue && (
        <Select.Item value={effectiveValue} disabled textValue={effectiveValue}>
          <span className={styles.trigger_only}>{effectiveValue}</span>
          <span className={styles.dropdown_only}>
            Unavailable: {effectiveValue}
          </span>
        </Select.Item>
      )}
      {allowUnset && (
        <Select.Item value={UNSET_MODEL_VALUE} textValue={unsetLabel}>
          <span className={styles.trigger_only}>{unsetLabel}</span>
          <span className={styles.dropdown_only}>{unsetLabel}</span>
        </Select.Item>
      )}
      {groupedModels.map((group) => (
        <Select.Group key={group.provider}>
          <Select.Label>{group.displayName}</Select.Label>
          {group.models.map(renderModelItem)}
        </Select.Group>
      ))}
    </Select.Content>
  );

  if (!caps || groupedModels.length === 0) {
    if (allowUnset) {
      return (
        <Flex
          direction={compact ? "row" : "column"}
          align={compact ? "center" : undefined}
          gap="1"
        >
          {showLabel && (
            <Text size="1" color="gray">
              {label}
            </Text>
          )}
          <Select.Root
            value={showUnavailableValue ? effectiveValue : UNSET_MODEL_VALUE}
            onValueChange={handleValueChange}
            disabled={disabled}
            size={compact ? "1" : "2"}
          >
            <Select.Trigger
              variant={compact ? "ghost" : undefined}
              className={compact ? styles.compactTrigger : undefined}
              style={compact ? undefined : { width: "100%" }}
            />
            <Select.Content position="popper">
              {showUnavailableValue && (
                <Select.Item
                  value={effectiveValue}
                  disabled
                  textValue={effectiveValue}
                >
                  <span className={styles.trigger_only}>{effectiveValue}</span>
                  <span className={styles.dropdown_only}>
                    Unavailable: {effectiveValue}
                  </span>
                </Select.Item>
              )}
              <Select.Item value={UNSET_MODEL_VALUE} textValue={unsetLabel}>
                <span className={styles.trigger_only}>{unsetLabel}</span>
                <span className={styles.dropdown_only}>{unsetLabel}</span>
              </Select.Item>
            </Select.Content>
          </Select.Root>
        </Flex>
      );
    }

    return (
      <Text size="1" color="gray" style={{ lineHeight: 1 }}>
        {showLabel ? `${label} ` : ""}
        {triggerLabel || "No models"}
      </Text>
    );
  }

  if (compact) {
    return (
      <Flex align="center" gap="1">
        {showLabel && (
          <Text size="1" color="gray" style={{ lineHeight: 1 }}>
            {label}
          </Text>
        )}
        <Select.Root
          value={selectValue}
          onValueChange={handleValueChange}
          disabled={disabled}
          size="1"
        >
          <Select.Trigger
            variant="ghost"
            className={styles.compactTrigger}
            title={
              disabled
                ? "Cannot change model while streaming"
                : "Click to change model"
            }
            style={{
              cursor: disabled ? "not-allowed" : "pointer",
              opacity: disabled ? 0.5 : 1,
            }}
          />
          {renderContent()}
        </Select.Root>
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="1">
      {showLabel && (
        <Text size="1" color="gray">
          {label}
        </Text>
      )}
      <Select.Root
        value={selectValue}
        onValueChange={handleValueChange}
        disabled={disabled}
        size="2"
      >
        <Select.Trigger style={{ width: "100%" }} />
        {renderContent()}
      </Select.Root>
    </Flex>
  );
};
