import React, {
  useCallback,
  useMemo,
  useState,
  useRef,
  useEffect,
} from "react";
import { Flex, Text, Popover, Separator, Badge } from "@radix-ui/themes";
import { ChevronDownIcon } from "@radix-ui/react-icons";
import { useAppDispatch, useCapsForToolUse } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import styles from "./ChatSettingsDropdown.module.css";

export type ModelPickerPopoverProps = {
  value: string;
  onValueChange: (model: string) => void;
  disabled?: boolean;
};

export const ModelPickerPopover: React.FC<ModelPickerPopoverProps> = ({
  value,
  onValueChange,
  disabled,
}) => {
  const dispatch = useAppDispatch();
  const caps = useCapsForToolUse();
  const [isOpen, setIsOpen] = useState(false);
  const selectedModelRef = useRef<HTMLButtonElement>(null);
  const modelListRef = useRef<HTMLDivElement>(null);

  const groupedModels = useMemo(() => {
    return enrichAndGroupModels(caps.usableModelsForPlan, caps.data);
  }, [caps.usableModelsForPlan, caps.data]);

  useEffect(() => {
    if (!isOpen) return;

    const scrollToSelected = () => {
      const container = modelListRef.current;
      const selected = selectedModelRef.current;
      if (container && selected && container.clientHeight > 0) {
        const containerHeight = container.clientHeight;
        const selectedTop = selected.offsetTop;
        const selectedHeight = selected.offsetHeight;
        container.scrollTop =
          selectedTop - containerHeight / 2 + selectedHeight / 2;
        return true;
      }
      return false;
    };

    let attempts = 0;
    const maxAttempts = 10;
    const tryScroll = () => {
      if (scrollToSelected() || attempts >= maxAttempts) return;
      attempts++;
      requestAnimationFrame(tryScroll);
    };

    requestAnimationFrame(tryScroll);
  }, [isOpen]);

  const handleModelSelect = useCallback(
    (modelValue: string) => {
      if (modelValue === "add-new-model") {
        dispatch(push({ name: "providers page" }));
        return;
      }
      onValueChange(modelValue);
      setIsOpen(false);
    },
    [dispatch, onValueChange],
  );

  const displayName = value || "Select model";

  return (
    <Popover.Root open={isOpen} onOpenChange={setIsOpen}>
      <Popover.Trigger>
        <button
          className={`${styles.trigger} ${disabled ? styles.disabled : ""}`}
          disabled={disabled}
          type="button"
        >
          <Flex align="center" gap="1" className={styles.triggerContent}>
            <Text size="1" className={styles.modelName}>
              {displayName}
            </Text>
            <ChevronDownIcon className={styles.chevron} />
          </Flex>
        </button>
      </Popover.Trigger>

      <Popover.Content
        className={styles.content}
        side="top"
        align="start"
        sideOffset={8}
      >
        <div className={`${styles.section} ${styles.modelSection}`}>
          <div className={styles.modelList} ref={modelListRef}>
            {groupedModels.map((group, groupIndex) => (
              <React.Fragment key={group.provider}>
                {groupIndex > 0 && (
                  <Separator size="4" className={styles.groupSeparator} />
                )}
                <Text size="1" color="gray" className={styles.groupHeader}>
                  {group.displayName}
                </Text>
                {group.models.map((model) => {
                  const isSelected = value === model.value;
                  return (
                    <button
                      key={model.value}
                      ref={isSelected ? selectedModelRef : undefined}
                      className={`${styles.item} ${
                        isSelected ? styles.itemSelected : ""
                      } ${model.disabled ? styles.itemDisabled : ""}`}
                      onClick={() => handleModelSelect(model.value)}
                      disabled={model.disabled}
                      type="button"
                    >
                      <Flex align="center" gap="1">
                        <Text
                          size="1"
                          weight="medium"
                          className={styles.itemModelName}
                        >
                          {model.value}
                        </Text>
                        {model.isDefault && (
                          <Badge
                            size="1"
                            color="blue"
                            variant="soft"
                            className={styles.badge}
                          >
                            Default
                          </Badge>
                        )}
                        {model.isThinking && (
                          <Badge
                            size="1"
                            color="purple"
                            variant="soft"
                            className={styles.badge}
                          >
                            Reasoning
                          </Badge>
                        )}
                      </Flex>
                    </button>
                  );
                })}
              </React.Fragment>
            ))}
            <Separator size="4" className={styles.groupSeparator} />
            <button
              className={styles.item}
              onClick={() => handleModelSelect("add-new-model")}
              type="button"
            >
              <Text size="1">Add new model...</Text>
            </button>
          </div>
        </div>
      </Popover.Content>
    </Popover.Root>
  );
};
