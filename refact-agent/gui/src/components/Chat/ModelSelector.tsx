import React, { useMemo } from "react";
import { Select, Text, Flex } from "@radix-ui/themes";
import { useCapsForToolUse } from "../../hooks";
import { useGetCapsQuery } from "../../services/refact/caps";
import { RichModelSelectItem } from "../Select/RichModelSelectItem";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import styles from "../Select/select.module.css";

export type ModelSelectorProps = {
  disabled?: boolean;
  value?: string;
  onValueChange?: (model: string) => void;
  label?: string;
  showLabel?: boolean;
  compact?: boolean;
};

export const ModelSelector: React.FC<ModelSelectorProps> = ({
  disabled,
  value,
  onValueChange,
  label = "model:",
  showLabel = true,
  compact = true,
}) => {
  const isControlled = onValueChange !== undefined || value !== undefined;
  const capsForToolUse = useCapsForToolUse();
  const { data: caps } = useGetCapsQuery(undefined);

  const capsData = isControlled ? caps : capsForToolUse.data;

  const usableModels = useMemo(() => {
    if (isControlled && capsData) {
      return Object.keys(capsData.chat_models).map((model) => ({
        value: model,
        textValue: model,
        disabled: false,
      }));
    }
    return capsForToolUse.usableModelsForPlan;
  }, [isControlled, capsData, capsForToolUse.usableModelsForPlan]);

  const groupedModels = useMemo(
    () => enrichAndGroupModels(usableModels, capsData),
    [usableModels, capsData],
  );

  const defaultModel = capsData?.chat_default_model ?? "";
  const effectiveValue = isControlled
    ? value ?? defaultModel
    : capsForToolUse.currentModel;
  const handleChange = isControlled
    ? (model: string) => onValueChange?.(model)
    : capsForToolUse.setCapModel;
  const currentModelName = effectiveValue.replace(/^refact\//, "");

  if (!capsData || groupedModels.length === 0) {
    return (
      <Text size="1" color="gray" style={{ lineHeight: 1 }}>
        {showLabel ? `${label} ` : ""}
        {currentModelName || "No models"}
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
          value={effectiveValue}
          onValueChange={handleChange}
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
          <Select.Content position="popper">
            {groupedModels.map((group) => (
              <Select.Group key={group.provider}>
                <Select.Label>{group.displayName}</Select.Label>
                {group.models.map((model) => (
                  <Select.Item
                    key={model.value}
                    value={model.value}
                    disabled={model.disabled}
                    textValue={model.displayName}
                  >
                    <span className={styles.trigger_only}>
                      {model.displayName}
                    </span>
                    <span className={styles.dropdown_only}>
                      <RichModelSelectItem
                        displayName={model.displayName}
                        pricing={model.pricing}
                        nCtx={model.nCtx}
                        capabilities={model.capabilities}
                        isDefault={model.isDefault}
                        isThinking={model.isThinking}
                        isLight={model.isLight}
                      />
                    </span>
                  </Select.Item>
                ))}
              </Select.Group>
            ))}
          </Select.Content>
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
        value={effectiveValue}
        onValueChange={handleChange}
        disabled={disabled}
        size="2"
      >
        <Select.Trigger style={{ width: "100%" }} />
        <Select.Content position="popper">
          {groupedModels.map((group) => (
            <Select.Group key={group.provider}>
              <Select.Label>{group.displayName}</Select.Label>
              {group.models.map((model) => (
                <Select.Item
                  key={model.value}
                  value={model.value}
                  disabled={model.disabled}
                  textValue={model.displayName}
                >
                  <span className={styles.trigger_only}>
                    {model.displayName}
                  </span>
                  <span className={styles.dropdown_only}>
                    <RichModelSelectItem
                      displayName={model.displayName}
                      pricing={model.pricing}
                      nCtx={model.nCtx}
                      capabilities={model.capabilities}
                      isDefault={model.isDefault}
                      isThinking={model.isThinking}
                      isLight={model.isLight}
                    />
                  </span>
                </Select.Item>
              ))}
            </Select.Group>
          ))}
        </Select.Content>
      </Select.Root>
    </Flex>
  );
};
