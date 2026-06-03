import React, { useCallback, useMemo } from "react";
import { Text, Flex, Skeleton, Box } from "@radix-ui/themes";
import { Select, type SelectProps } from "../Select";
import { useCapsForToolUse } from "../../hooks";
import { useAppDispatch } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { RichModelSelectItem } from "../Select/RichModelSelectItem";
import { enrichAndGroupModels } from "../../utils/enrichModels";

export const CapsSelect: React.FC<{ disabled?: boolean }> = ({ disabled }) => {
  const caps = useCapsForToolUse();
  const dispatch = useAppDispatch();

  const handleAddNewModelClick = useCallback(() => {
    dispatch(push({ name: "providers page" }));
  }, [dispatch]);

  const onSelectChange = useCallback(
    (value: string) => {
      if (value === "add-new-model") {
        handleAddNewModelClick();
        return;
      }
      caps.setCapModel(value);
    },
    [handleAddNewModelClick, caps],
  );

  const optionsWithToolTips: SelectProps["options"] = useMemo(() => {
    const groupedModels = enrichAndGroupModels(
      caps.usableModelsForPlan,
      caps.data,
    );

    if (groupedModels.length === 0) {
      return [
        ...caps.usableModelsForPlan,
        { type: "separator" },
        {
          value: "add-new-model",
          textValue: "Add new model",
        },
      ];
    }

    const flatOptions: SelectProps["options"] = [];
    groupedModels.forEach((group, index) => {
      if (index > 0) {
        flatOptions.push({ type: "separator" });
      }
      group.models.forEach((model) => {
        flatOptions.push({
          value: model.value,
          textValue: model.displayName,
          disabled: model.disabled,
          children: (
            <RichModelSelectItem
              displayName={model.displayName}
              pricing={model.pricing}
              nCtx={model.nCtx}
              capabilities={model.capabilities}
              isDefault={model.isDefault}
              isChat2={model.isChat2}
              isTaskPlannerAgent={model.isTaskPlannerAgent}
              isThinking={model.isThinking}
              isLight={model.isLight}
              isBuddy={model.isBuddy}
            />
          ),
        });
      });
    });

    return [
      ...flatOptions,
      { type: "separator" },
      {
        value: "add-new-model",
        textValue: "Add new model",
      },
    ];
  }, [caps.data, caps.usableModelsForPlan]);

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
            <Select
              title="chat model"
              options={optionsWithToolTips}
              value={caps.currentModel}
              onChange={onSelectChange}
              disabled={disabled}
            />
          )}
        </Box>
      </Skeleton>
    </Flex>
  );
};
