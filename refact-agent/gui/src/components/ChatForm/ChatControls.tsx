import React, { useCallback, useMemo, useState } from "react";
import { Text, Flex, Skeleton, Box } from "@radix-ui/themes";
import { ChevronDown } from "lucide-react";
import { useCapsForToolUse } from "../../hooks";
import { useAppDispatch } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import {
  formatContextWindow,
  formatPricing,
} from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import { CapabilityIcons } from "../../features/Providers/ProviderForm/ProviderModelsList/components";
import {
  Button,
  ModelSelector,
  Popover,
  type ModelOption,
  type ModelSelectorBadge,
  type ModelSelectorGroup,
} from "../ui";

function modelBadges(model: {
  isDefault?: boolean;
  isThinking?: boolean;
  isLight?: boolean;
  isBuddy?: boolean;
  isTaskPlannerAgent?: boolean;
  isChat2?: boolean;
}): ModelSelectorBadge[] {
  return [
    model.isDefault ? "default" : null,
    model.isThinking ? "reasoning" : null,
    model.isLight ? "light" : null,
    model.isBuddy ? "buddy" : null,
    model.isTaskPlannerAgent ? "task-agent" : null,
    model.isChat2 ? "chat2" : null,
  ].filter((badge): badge is ModelSelectorBadge => badge !== null);
}

export const CapsSelect: React.FC<{ disabled?: boolean }> = ({ disabled }) => {
  const caps = useCapsForToolUse();
  const dispatch = useAppDispatch();
  const [isOpen, setIsOpen] = useState(false);

  const handleAddNewModelClick = useCallback(() => {
    dispatch(push({ name: "providers page" }));
  }, [dispatch]);

  const onSelectChange = useCallback(
    (value: string) => {
      caps.setCapModel(value);
      setIsOpen(false);
    },
    [caps],
  );

  const groupedModels = useMemo(() => {
    return enrichAndGroupModels(caps.usableModelsForPlan, caps.data);
  }, [caps.data, caps.usableModelsForPlan]);

  const groups = useMemo<ModelSelectorGroup[]>(() => {
    return groupedModels.map((group) => ({
      id: group.provider,
      label: group.displayName,
    }));
  }, [groupedModels]);

  const models = useMemo<ModelOption[]>(() => {
    return groupedModels.flatMap((group) =>
      group.models.map((model) => {
        const pricingParts = model.pricing
          ? formatPricing(model.pricing, true).split("/")
          : null;

        return {
          value: model.value,
          displayName: model.value,
          group: group.provider,
          disabled: model.disabled,
          badges: modelBadges(model),
          pricing: pricingParts
            ? {
                prompt: pricingParts[0],
                output: pricingParts[1],
              }
            : undefined,
          contextWindow: model.nCtx
            ? formatContextWindow(model.nCtx)
            : undefined,
          capabilities: model.capabilities ? (
            <CapabilityIcons capabilities={model.capabilities} />
          ) : undefined,
        };
      }),
    );
  }, [groupedModels]);

  const allDisabled = caps.usableModelsForPlan.every((option) => {
    if (typeof option === "string") return false;
    return option.disabled;
  });

  return (
    <Flex gap="2" align="center" wrap="wrap">
      <Skeleton loading={caps.loading}>
        <Box>
          {allDisabled ? (
            <Text size="1" color="gray">
              No models available
            </Text>
          ) : (
            <Popover open={isOpen} onOpenChange={setIsOpen} responsive={false}>
              <Popover.Trigger asChild>
                <Button
                  aria-label="chat model"
                  disabled={disabled}
                  rightIcon={ChevronDown}
                  type="button"
                  variant="soft"
                >
                  {caps.currentModel || "Select model"}
                </Button>
              </Popover.Trigger>
              <Popover.Content
                align="start"
                maxHeight="min(520px, calc(100dvh - var(--rf-space-6)))"
                maxWidth="420px"
                scrollable={false}
                side="bottom"
                sideOffset={8}
              >
                <ModelSelector
                  disabled={disabled}
                  groups={groups}
                  models={models}
                  value={caps.currentModel}
                  variant="inline"
                  onAddNewModel={handleAddNewModelClick}
                  onSelect={onSelectChange}
                />
              </Popover.Content>
            </Popover>
          )}
        </Box>
      </Skeleton>
    </Flex>
  );
};
