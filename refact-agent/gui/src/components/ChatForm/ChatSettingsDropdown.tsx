import React, {
  useCallback,
  useMemo,
  useState,
  useRef,
  useEffect,
} from "react";
import {
  Flex,
  Text,
  Popover,
  Separator,
  Switch,
  Skeleton,
  Slider,
  Badge,
} from "@radix-ui/themes";
import { ChevronDownIcon } from "@radix-ui/react-icons";
import { useAppSelector, useAppDispatch, useCapsForToolUse } from "../../hooks";
import { useGetCapsQuery, CapCost } from "../../services/refact/caps";
import {
  selectChatId,
  selectContextTokensCap,
  selectModel,
  selectIsStreaming,
  selectIsWaiting,
  selectThreadBoostReasoning,
  setContextTokensCap,
} from "../../features/Chat/Thread";
import { push } from "../../features/Pages/pagesSlice";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import { useThinking } from "../../hooks/useThinking";
import { formatContextWindow } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import styles from "./ChatSettingsDropdown.module.css";

const CAP_STEPS = [16000, 32000, 64000, 128000, 200000, 256000];
const MIN_CAP = 16000;

function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(tokens % 1000000 === 0 ? 0 : 1)}M`;
  }
  return `${Math.round(tokens / 1000)}K`;
}

function formatCoins(coins: number | null): string {
  if (coins === null) return "–";
  if (coins >= 1000) {
    return `${(coins / 1000).toFixed(coins % 1000 === 0 ? 0 : 1)}k`;
  }
  return coins.toString();
}

function formatPricingDetailed(cost: CapCost): {
  prompt: string;
  output: string;
} {
  const toCoins = (n?: number) =>
    typeof n === "number" && Number.isFinite(n) ? Math.round(n * 1000) : null;

  return {
    prompt: formatCoins(toCoins(cost.prompt)),
    output: formatCoins(toCoins(cost.generated)),
  };
}

function getSliderSteps(maxTokens: number): number[] {
  const steps = CAP_STEPS.filter((s) => s <= maxTokens);
  if (!steps.includes(maxTokens)) {
    steps.push(maxTokens);
  }
  return steps.sort((a, b) => a - b);
}

function valueToSliderPosition(value: number, steps: number[]): number {
  const idx = steps.findIndex((s) => s >= value);
  if (idx === -1) return steps.length - 1;
  if (steps[idx] === value) return idx;
  if (idx === 0) return 0;
  const prev = steps[idx - 1];
  const next = steps[idx];
  const ratio = (value - prev) / (next - prev);
  return idx - 1 + ratio;
}

function sliderPositionToValue(position: number, steps: number[]): number {
  const idx = Math.floor(position);
  if (idx >= steps.length - 1) return steps[steps.length - 1];
  const frac = position - idx;
  if (frac === 0) return steps[idx];
  return Math.round(steps[idx] + frac * (steps[idx + 1] - steps[idx]));
}

export const ChatSettingsDropdown: React.FC = () => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const contextCap = useAppSelector(selectContextTokensCap);
  const threadModel = useAppSelector(selectModel);
  const isBoostReasoningEnabled = useAppSelector(selectThreadBoostReasoning);

  const caps = useCapsForToolUse();
  const capsQuery = useGetCapsQuery(undefined);

  const {
    handleReasoningChange,
    shouldBeDisabled: thinkingDisabled,
    supportsBoostReasoning,
    areCapsInitialized,
  } = useThinking();

  const isInteractionDisabled = isStreaming || isWaiting;

  // Model data
  const currentModelName = caps.currentModel || "Select model";
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

  const selectedModelDetail = useMemo(() => {
    if (!caps.currentModel) return null;
    const data = capsQuery.data;
    if (!data?.chat_models) return null;
    const modelData = data.chat_models[caps.currentModel] as
      | { n_ctx: number }
      | undefined;
    if (!modelData) return null;
    const pricing =
      data.metadata?.pricing?.[caps.currentModel.replace(/^refact\//, "")];
    return {
      nCtx: modelData.n_ctx,
      pricing: pricing ? formatPricingDetailed(pricing) : null,
    };
  }, [caps.currentModel, capsQuery.data]);

  // Context cap data
  const maxTokens = useMemo(() => {
    const chatModels = capsQuery.data?.chat_models;
    if (!chatModels || !threadModel) return 0;
    const modelData = chatModels[threadModel];
    return modelData.n_ctx;
  }, [capsQuery.data, threadModel]);

  const sliderSteps = useMemo(() => getSliderSteps(maxTokens), [maxTokens]);

  const effectiveCap = useMemo(() => {
    if (!contextCap || contextCap > maxTokens) return maxTokens;
    if (contextCap < MIN_CAP) return MIN_CAP;
    return contextCap;
  }, [contextCap, maxTokens]);

  const [localSliderValue, setLocalSliderValue] = useState<number | null>(null);
  const displayCap = localSliderValue ?? effectiveCap;

  // Handlers
  const handleModelSelect = useCallback(
    (modelValue: string) => {
      if (modelValue === "add-new-model") {
        dispatch(push({ name: "providers page" }));
        return;
      }
      caps.setCapModel(modelValue);
    },
    [caps, dispatch],
  );

  const handleSliderChange = useCallback(
    (values: number[]) => {
      const newValue = sliderPositionToValue(values[0], sliderSteps);
      setLocalSliderValue(newValue);
    },
    [sliderSteps],
  );

  const handleSliderCommit = useCallback(
    (values: number[]) => {
      const newValue = sliderPositionToValue(values[0], sliderSteps);
      dispatch(setContextTokensCap({ chatId, value: newValue }));
      setLocalSliderValue(null);
    },
    [dispatch, chatId, sliderSteps],
  );

  const noop = useCallback(() => {
    /* intentionally empty */
  }, []);
  const handleThinkingToggle = useCallback(
    (checked: boolean) => {
      handleReasoningChange(
        {
          preventDefault: noop,
          stopPropagation: noop,
        } as unknown as React.MouseEvent<HTMLButtonElement>,
        checked,
      );
    },
    [handleReasoningChange, noop],
  );

  // Loading state
  if (caps.loading || !areCapsInitialized) {
    return (
      <Skeleton>
        <div className={styles.trigger}>
          <Text size="1">Loading...</Text>
          <ChevronDownIcon />
        </div>
      </Skeleton>
    );
  }

  // Trigger display
  const triggerContent = (
    <Flex align="center" gap="1" className={styles.triggerContent}>
      <Text size="1" className={styles.modelName}>
        {currentModelName}
      </Text>
      {maxTokens > 0 && (
        <>
          <Text size="1" color="gray">
            ·
          </Text>
          <Text size="1" color="gray">
            {formatTokens(effectiveCap)}
          </Text>
        </>
      )}
      {supportsBoostReasoning && isBoostReasoningEnabled && (
        <>
          <Text size="1" color="gray">
            ·
          </Text>
          <Text size="1">🧠</Text>
        </>
      )}
      <ChevronDownIcon className={styles.chevron} />
    </Flex>
  );

  return (
    <Popover.Root open={isOpen} onOpenChange={setIsOpen}>
      <Popover.Trigger>
        <button
          className={`${styles.trigger} ${
            isInteractionDisabled ? styles.disabled : ""
          }`}
          disabled={isInteractionDisabled}
          type="button"
        >
          {triggerContent}
        </button>
      </Popover.Trigger>

      <Popover.Content
        className={styles.content}
        side="top"
        align="start"
        sideOffset={8}
      >
        {/* Model Section */}
        <div className={styles.section}>
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
                  const isSelected = caps.currentModel === model.value;
                  return (
                    <button
                      key={model.value}
                      ref={isSelected ? selectedModelRef : undefined}
                      className={`${styles.item} ${
                        isSelected ? styles.itemSelected : ""
                      } ${model.disabled ? styles.itemDisabled : ""}`}
                      onClick={() => handleModelSelect(model.value)}
                      disabled={isInteractionDisabled || model.disabled}
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

        {/* Model Details */}
        {selectedModelDetail &&
          (selectedModelDetail.nCtx || selectedModelDetail.pricing) && (
            <>
              <Separator size="4" />
              <Flex gap="2" align="center" px="2" py="1">
                {selectedModelDetail.nCtx && (
                  <Text size="1" color="gray">
                    {formatContextWindow(selectedModelDetail.nCtx)} context
                  </Text>
                )}
                {selectedModelDetail.pricing && (
                  <>
                    <Text size="1" color="gray">
                      ·
                    </Text>
                    <Text size="1" color="gray">
                      {selectedModelDetail.pricing.prompt}/
                      {selectedModelDetail.pricing.output} ⓒ/1K tokens
                    </Text>
                  </>
                )}
              </Flex>
            </>
          )}

        <Separator size="4" />

        {/* Context Cap Section with Slider */}
        {sliderSteps.length > 1 && (
          <>
            <div className={styles.section}>
              <Flex justify="between" align="center" mb="2">
                <Text
                  size="1"
                  color="gray"
                  weight="medium"
                  className={styles.sectionHeader}
                >
                  Context window
                </Text>
                <Text size="1" weight="medium">
                  {formatTokens(displayCap)}
                  {displayCap === maxTokens && " (max)"}
                </Text>
              </Flex>
              <Flex align="center" gap="2" className={styles.sliderContainer}>
                <Text size="1" color="gray">
                  {formatTokens(MIN_CAP)}
                </Text>
                <Slider
                  size="1"
                  min={0}
                  max={sliderSteps.length - 1}
                  step={0.01}
                  value={[valueToSliderPosition(displayCap, sliderSteps)]}
                  onValueChange={handleSliderChange}
                  onValueCommit={handleSliderCommit}
                  disabled={isInteractionDisabled}
                  className={styles.slider}
                />
                <Text size="1" color="gray">
                  {formatTokens(maxTokens)}
                </Text>
              </Flex>
            </div>
            <Separator size="4" />
          </>
        )}

        {/* Thinking Section */}
        {supportsBoostReasoning && (
          <div className={styles.section}>
            <Flex align="center" justify="between" gap="3">
              <Flex align="center" gap="1">
                <Text size="1">🧠</Text>
                <Text size="1" weight="medium">
                  Extended reasoning
                </Text>
              </Flex>
              <Switch
                size="1"
                checked={isBoostReasoningEnabled}
                onCheckedChange={handleThinkingToggle}
                disabled={thinkingDisabled}
              />
            </Flex>
          </div>
        )}
      </Popover.Content>
    </Popover.Root>
  );
};

ChatSettingsDropdown.displayName = "ChatSettingsDropdown";
