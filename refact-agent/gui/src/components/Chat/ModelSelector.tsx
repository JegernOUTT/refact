import React, { useMemo } from "react";
import { Select, Text, Flex } from "@radix-ui/themes";
import { useCapsForToolUse } from "../../hooks";
import { useGetCapsQuery } from "../../services/refact/caps";
import { RichModelSelectItem } from "../Select/RichModelSelectItem";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import styles from "../Select/select.module.css";

export type ModelSelectorProps = {
  disabled?: boolean;
  value: string | undefined;
  onValueChange: (model: string) => void;
  label?: string;
  showLabel?: boolean;
  compact?: boolean;
  defaultValue?: string;
};

export const ModelSelector: React.FC<ModelSelectorProps> = ({
  disabled,
  value,
  onValueChange,
  label = "model:",
  showLabel = true,
  compact = true,
  defaultValue,
}) => {
  const capsForToolUse = useCapsForToolUse();
  const { data: caps } = useGetCapsQuery(undefined);

  const capsData = caps ?? capsForToolUse.data;

  const usableModels = capsForToolUse.usableModelsForPlan;

  const groupedModels = useMemo(
    () => enrichAndGroupModels(usableModels, capsData),
    [usableModels, capsData],
  );

  const defaultModel = defaultValue ?? capsData?.chat_default_model ?? "";
  const effectiveValue = value ?? defaultModel;
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
          onValueChange={onValueChange}
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
                    textValue={model.value}
                  >
                    <span className={styles.trigger_only}>{model.value}</span>
                    <span className={styles.dropdown_only}>
                      <RichModelSelectItem
                        displayName={model.value}
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
        onValueChange={onValueChange}
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
                  textValue={model.value}
                >
                  <span className={styles.trigger_only}>{model.value}</span>
                  <span className={styles.dropdown_only}>
                    <RichModelSelectItem
                      displayName={model.value}
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
