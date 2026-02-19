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
  Skeleton,
  Slider,
  Badge,
  Switch,
  Callout,
} from "@radix-ui/themes";
import { ChevronDownIcon, ChevronRightIcon } from "@radix-ui/react-icons";
import * as Collapsible from "@radix-ui/react-collapsible";
import { useAppSelector, useAppDispatch, useCapsForToolUse } from "../../hooks";
import { useGetCapsQuery, CapCost } from "../../services/refact/caps";
import {
  selectChatId,
  selectContextTokensCap,
  selectModel,
  selectMessages,
  selectIsStreaming,
  selectIsWaiting,
  selectThreadBoostReasoning,
  selectReasoningEffort,
  selectThinkingBudget,
  selectTemperature,
  selectMaxTokens,
  setContextTokensCap,
  setReasoningEffort,
  setThinkingBudget,
  setTemperature,
  setMaxTokens,
} from "../../features/Chat/Thread";
import type { ReasoningEffort } from "../../features/Chat/Thread/types";
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

function formatUsdPrice(price: number | undefined): string {
  if (typeof price !== "number" || !Number.isFinite(price)) return "–";
  if (price >= 100) {
    return `$${price.toFixed(0)}`;
  }
  if (price >= 10) {
    return `$${price.toFixed(1)}`;
  }
  return `$${price.toFixed(2)}`;
}

function formatPricingDetailed(cost: CapCost): {
  prompt: string;
  output: string;
} {
  return {
    prompt: formatUsdPrice(cost.prompt),
    output: formatUsdPrice(cost.generated),
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
  const messages = useAppSelector(selectMessages);
  const isBoostReasoningEnabled = useAppSelector(selectThreadBoostReasoning);
  const threadTemperature = useAppSelector(selectTemperature);
  const threadMaxTokens = useAppSelector(selectMaxTokens);
  const threadReasoningEffort = useAppSelector(selectReasoningEffort);
  const threadThinkingBudget = useAppSelector(selectThinkingBudget);
  const hasAnyReasoningConfigured =
    (isBoostReasoningEnabled ?? false) ||
    threadReasoningEffort != null ||
    threadThinkingBudget != null;

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
  const [advancedOpen, setAdvancedOpen] = useState(false);
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
      | {
          n_ctx: number;
          default_temperature?: number;
          default_max_tokens?: number;
          max_output_tokens?: number;
          reasoning_effort_options?: string[] | null;
          supports_thinking_budget?: boolean;
          supports_adaptive_thinking_budget?: boolean;
        }
      | undefined;
    if (!modelData) return null;
    const pricing =
      data.metadata?.pricing?.[caps.currentModel.replace(/^refact\//, "")];
    return {
      nCtx: modelData.n_ctx,
      defaultTemperature: modelData.default_temperature,
      defaultMaxTokens: modelData.default_max_tokens,
      maxOutputTokens: modelData.max_output_tokens,
      reasoningEffortOptions: modelData.reasoning_effort_options,
      supportsThinkingBudget: modelData.supports_thinking_budget,
      supportsAdaptiveThinkingBudget:
        modelData.supports_adaptive_thinking_budget,
      pricing: pricing ? formatPricingDetailed(pricing) : null,
    };
  }, [caps.currentModel, capsQuery.data]);

  const maxTokens = useMemo(() => {
    const chatModels = capsQuery.data?.chat_models;
    if (!chatModels || !threadModel) return 0;
    if (!Object.prototype.hasOwnProperty.call(chatModels, threadModel))
      return 0;
    return chatModels[threadModel].n_ctx;
  }, [capsQuery.data, threadModel]);

  const sliderSteps = useMemo(() => getSliderSteps(maxTokens), [maxTokens]);

  const effectiveCap = useMemo(() => {
    if (!contextCap || contextCap > maxTokens) return maxTokens;
    if (contextCap < MIN_CAP) return MIN_CAP;
    return contextCap;
  }, [contextCap, maxTokens]);

  const [localSliderValue, setLocalSliderValue] = useState<number | null>(null);
  const displayCap = localSliderValue ?? effectiveCap;

  const [localTemperature, setLocalTemperature] = useState<number | null>(null);
  const [localThinkingBudget, setLocalThinkingBudget] = useState<number | null>(
    null,
  );
  const [localMaxTokens, setLocalMaxTokens] = useState<number | null>(null);
  const displayTemperature = localTemperature ?? threadTemperature;
  const displayThinkingBudget = localThinkingBudget ?? threadThinkingBudget;
  const displayMaxTokens = localMaxTokens ?? threadMaxTokens;

  const isStartedChat = messages.length > 0;

  useEffect(() => {
    setLocalSliderValue(null);
    setLocalTemperature(null);
    setLocalThinkingBudget(null);
    setLocalMaxTokens(null);
  }, [chatId]);

  useEffect(() => {
    if (!isOpen) {
      setLocalSliderValue(null);
      setLocalTemperature(null);
      setLocalThinkingBudget(null);
      setLocalMaxTokens(null);
    }
  }, [isOpen]);

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

      if (checked) {
        // Reasoning requires temperature to be unset (None).
        // Dispatch explicitly so the setTemperature middleware + persistence
        // listeners fire, keeping Redux, backend, and localStorage in sync.
        dispatch(setTemperature({ chatId, value: null }));
      } else {
        // Ensure "Reasoning" toggle truly controls reasoning.
        // Backend treats `reasoning_effort` / `thinking_budget` as enabling reasoning
        // even if `boost_reasoning` is turned off.
        dispatch(setReasoningEffort({ chatId, value: null }));
        dispatch(setThinkingBudget({ chatId, value: null }));
      }
    },
    [handleReasoningChange, noop, dispatch, chatId],
  );

  const handleTemperatureChange = useCallback((values: number[]) => {
    setLocalTemperature(values[0]);
  }, []);

  const handleTemperatureCommit = useCallback(
    (values: number[]) => {
      if (hasAnyReasoningConfigured) {
        // UI should be disabled already, but keep commit a no-op defensively.
        setLocalTemperature(null);
        return;
      }
      dispatch(setTemperature({ chatId, value: values[0] }));
      setLocalTemperature(null);
    },
    [dispatch, chatId, hasAnyReasoningConfigured],
  );

  const handleTemperatureReset = useCallback(() => {
    if (hasAnyReasoningConfigured) return;
    dispatch(setTemperature({ chatId, value: null }));
    setLocalTemperature(null);
  }, [dispatch, chatId, hasAnyReasoningConfigured]);

  const handleMaxTokensReset = useCallback(() => {
    dispatch(setMaxTokens({ chatId, value: null }));
    setLocalMaxTokens(null);
  }, [dispatch, chatId]);

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
        className={`${styles.content} ${
          advancedOpen ? styles.contentAdvancedOpen : ""
        }`}
        side="top"
        align="start"
        sideOffset={8}
      >
        {/* Model Section */}
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
                      {selectedModelDetail.pricing.output} per 1M tokens
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
                  Reasoning
                </Text>
              </Flex>
              <Switch
                size="1"
                checked={isBoostReasoningEnabled}
                onCheckedChange={handleThinkingToggle}
                disabled={thinkingDisabled}
              />
            </Flex>

            {isStartedChat && (
              <Callout.Root color="amber" size="1" mt="2">
                <Callout.Text>
                  Changing reasoning mid-chat may break prompt caching (if
                  enabled) and make the next turn much more expensive.
                </Callout.Text>
              </Callout.Root>
            )}

            {isBoostReasoningEnabled && selectedModelDetail && (
              <>
                {/* Reasoning effort options (transparent) */}
                {selectedModelDetail.reasoningEffortOptions &&
                  selectedModelDetail.reasoningEffortOptions.length > 0 && (
                    <Flex align="center" justify="between" gap="2" mt="2">
                      <Text size="1" color="gray">
                        Effort
                      </Text>
                      <Flex gap="1">
                        {selectedModelDetail.reasoningEffortOptions.map(
                          (level) => (
                            <button
                              key={level}
                              type="button"
                              className={`${styles.effortButton} ${
                                (threadReasoningEffort ?? "medium") === level
                                  ? styles.effortButtonActive
                                  : ""
                              }`}
                              onClick={() =>
                                dispatch(
                                  setReasoningEffort({
                                    chatId,
                                    value: level as ReasoningEffort,
                                  }),
                                )
                              }
                              disabled={isInteractionDisabled}
                            >
                              <Text size="1">{level}</Text>
                            </button>
                          ),
                        )}
                      </Flex>
                    </Flex>
                  )}
                {/* Thinking budget slider */}
                {selectedModelDetail.supportsThinkingBudget && (
                  <Flex direction="column" gap="1" mt="2">
                    <Flex align="center" justify="between">
                      <Text size="1" color="gray">
                        Thinking tokens
                      </Text>
                      <Text size="1" weight="medium">
                        {displayThinkingBudget ?? 16384}
                      </Text>
                    </Flex>
                    <Flex align="center" gap="2">
                      <Text size="1" color="gray">
                        1K
                      </Text>
                      <Slider
                        size="1"
                        min={1024}
                        max={32768}
                        step={1024}
                        value={[displayThinkingBudget ?? 16384]}
                        onValueChange={(values) =>
                          setLocalThinkingBudget(values[0])
                        }
                        onValueCommit={(values) => {
                          dispatch(
                            setThinkingBudget({ chatId, value: values[0] }),
                          );
                          setLocalThinkingBudget(null);
                        }}
                        disabled={isInteractionDisabled}
                      />
                      <Text size="1" color="gray">
                        32K
                      </Text>
                    </Flex>
                  </Flex>
                )}
              </>
            )}
          </div>
        )}

        <Separator size="4" />

        {/* Advanced Settings Section */}
        <Collapsible.Root open={advancedOpen} onOpenChange={setAdvancedOpen}>
          <Collapsible.Trigger asChild>
            <button
              className={styles.advancedTrigger}
              type="button"
              disabled={isInteractionDisabled}
            >
              <Flex align="center" gap="1">
                <ChevronRightIcon
                  className={`${styles.advancedChevron} ${
                    advancedOpen ? styles.advancedChevronOpen : ""
                  }`}
                />
                <Text size="1" weight="medium">
                  Advanced settings
                </Text>
              </Flex>
            </button>
          </Collapsible.Trigger>
          <Collapsible.Content>
            <div className={styles.advancedContent}>
              {/* Temperature */}
              <div className={styles.advancedRow}>
                <Flex justify="between" align="center" mb="1">
                  <Text size="1" color="gray">
                    Temperature
                  </Text>
                  <Flex align="center" gap="2">
                    <Text size="1" weight="medium">
                      {hasAnyReasoningConfigured
                        ? "None"
                        : displayTemperature?.toFixed(1) ??
                          (selectedModelDetail?.defaultTemperature?.toFixed(
                            1,
                          ) ?? "0.7") + " (default)"}
                    </Text>
                    {threadTemperature != null && (
                      <button
                        type="button"
                        className={styles.resetButton}
                        onClick={handleTemperatureReset}
                        disabled={
                          isInteractionDisabled || hasAnyReasoningConfigured
                        }
                      >
                        ✕
                      </button>
                    )}
                  </Flex>
                </Flex>
                <Slider
                  size="1"
                  min={0}
                  max={2}
                  step={0.1}
                  value={[
                    displayTemperature ??
                      selectedModelDetail?.defaultTemperature ??
                      0.7,
                  ]}
                  onValueChange={handleTemperatureChange}
                  onValueCommit={handleTemperatureCommit}
                  disabled={isInteractionDisabled || hasAnyReasoningConfigured}
                />
              </div>

              {/* Max Tokens */}
              <div className={styles.advancedRow}>
                <Flex justify="between" align="center" mb="1">
                  <Text size="1" color="gray">
                    Max tokens
                  </Text>
                  <Flex align="center" gap="2">
                    <Text size="1" weight="medium">
                      {displayMaxTokens ??
                        (selectedModelDetail?.defaultMaxTokens
                          ? `${selectedModelDetail.defaultMaxTokens} (default)`
                          : "4096 (default)")}
                    </Text>
                    {threadMaxTokens != null && (
                      <button
                        type="button"
                        className={styles.resetButton}
                        onClick={handleMaxTokensReset}
                        disabled={isInteractionDisabled}
                      >
                        ✕
                      </button>
                    )}
                  </Flex>
                </Flex>
                <Flex align="center" gap="2">
                  <Text size="1" color="gray">
                    1K
                  </Text>
                  <Slider
                    size="1"
                    min={1024}
                    max={selectedModelDetail?.maxOutputTokens ?? 16384}
                    step={1024}
                    value={[
                      displayMaxTokens ??
                        selectedModelDetail?.defaultMaxTokens ??
                        4096,
                    ]}
                    onValueChange={(values) => setLocalMaxTokens(values[0])}
                    onValueCommit={(values) => {
                      dispatch(setMaxTokens({ chatId, value: values[0] }));
                      setLocalMaxTokens(null);
                    }}
                    disabled={isInteractionDisabled}
                  />
                  <Text size="1" color="gray">
                    {formatTokens(
                      selectedModelDetail?.maxOutputTokens ?? 16384,
                    )}
                  </Text>
                </Flex>
              </div>
            </div>
          </Collapsible.Content>
        </Collapsible.Root>
      </Popover.Content>
    </Popover.Root>
  );
};

ChatSettingsDropdown.displayName = "ChatSettingsDropdown";
