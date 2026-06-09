import React, { useCallback, useMemo, useState, useEffect } from "react";
import { Flex, Text, Separator, Skeleton } from "@radix-ui/themes";
import { Cross1Icon } from "@radix-ui/react-icons";
import {
  Brain,
  ChevronDown,
  Images,
  MousePointer2,
  Rocket,
  Wrench,
} from "lucide-react";
import classNames from "classnames";
import {
  useAppSelector,
  useAppDispatch,
  useCapsForToolUse,
  useGetCapsQuery,
} from "../../hooks";
import type { CapCost } from "../../services/refact/caps";
import {
  selectChatId,
  selectModel,
  selectMessages,
  selectIsStreaming,
  selectIsWaiting,
  selectThreadBoostReasoning,
  selectReasoningEffort,
  selectThinkingBudget,
  selectMaxTokens,
  setReasoningEffort,
  setThinkingBudget,
  setTemperature,
  setMaxTokens,
} from "../../features/Chat/Thread";
import type { ReasoningEffort } from "../../features/Chat/Thread/types";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import { useThinking } from "../../hooks/useThinking";
import { formatContextWindow } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import { ReasoningIcon } from "../../features/Providers/ProviderForm/ProviderModelsList/components/CapabilityIcons";
import type { ModelCapabilities } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import {
  Icon,
  ModelSelector as KitModelSelector,
  Popover as KitPopover,
  Slider,
  Switch,
} from "../ui";
import type { ModelOption, ModelSelectorBadge } from "../ui";
import styles from "./ChatSettingsDropdown.module.css";

const MIN_OUTPUT_TOKENS = 1024;

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

function CapabilityIcons({
  capabilities,
}: {
  capabilities?: ModelCapabilities;
}) {
  if (!capabilities) return null;

  return (
    <span className={styles.capabilityIcons}>
      {capabilities.supportsTools && (
        <Icon
          icon={Wrench}
          size="sm"
          tone="muted"
          aria-label="Supports tools"
        />
      )}
      {capabilities.supportsMultimodality && (
        <Icon
          icon={Images}
          size="sm"
          tone="muted"
          aria-label="Supports images"
        />
      )}
      {capabilities.supportsClicks && (
        <Icon
          icon={MousePointer2}
          size="sm"
          tone="muted"
          aria-label="Computer use"
        />
      )}
      {capabilities.supportsAgent && (
        <Icon icon={Rocket} size="sm" tone="muted" aria-label="Agent mode" />
      )}
      {(!!capabilities.reasoningEffortOptions?.length ||
        !!capabilities.supportsThinkingBudget ||
        !!capabilities.supportsAdaptiveThinkingBudget) && (
        <Icon icon={Brain} size="sm" tone="accent" aria-label="Reasoning" />
      )}
    </span>
  );
}

type ChatSettingsDropdownProps = {
  disabled?: boolean;
  compact?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export const ChatSettingsDropdown: React.FC<ChatSettingsDropdownProps> = ({
  disabled,
  compact = false,
  onOpenChange,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const threadModel = useAppSelector(selectModel);
  const messages = useAppSelector(selectMessages);
  const isBoostReasoningEnabled = useAppSelector(selectThreadBoostReasoning);
  const threadMaxTokens = useAppSelector(selectMaxTokens);
  const threadReasoningEffort = useAppSelector(selectReasoningEffort);
  const threadThinkingBudget = useAppSelector(selectThinkingBudget);

  const caps = useCapsForToolUse();
  const capsQuery = useGetCapsQuery(undefined);

  const {
    handleReasoningChange,
    shouldBeDisabled: thinkingDisabled,
    supportsBoostReasoning,
    areCapsInitialized,
  } = useThinking();

  const isInteractionDisabled = (disabled ?? false) || isStreaming || isWaiting;

  // Model data
  const currentModelName = caps.currentModel || "Select model";
  const [isOpen, setIsOpen] = useState(false);
  const handleOpenChange = useCallback(
    (open: boolean) => {
      setIsOpen(open);
      onOpenChange?.(open);
    },
    [onOpenChange],
  );
  const groupedModels = useMemo(() => {
    return enrichAndGroupModels(caps.usableModelsForPlan, caps.data);
  }, [caps.usableModelsForPlan, caps.data]);

  const modelSelectorGroups = useMemo(
    () =>
      groupedModels.map((group) => ({
        id: group.provider,
        label: group.displayName,
      })),
    [groupedModels],
  );

  const modelSelectorOptions = useMemo<ModelOption[]>(
    () =>
      groupedModels.flatMap((group) =>
        group.models.map((model) => ({
          value: model.value,
          displayName: model.value,
          group: group.provider,
          disabled: model.disabled || isInteractionDisabled,
          badges: modelBadges(model),
          pricing: model.pricing
            ? formatPricingDetailed(model.pricing)
            : undefined,
          contextWindow: model.nCtx
            ? formatContextWindow(model.nCtx)
            : undefined,
          capabilities: <CapabilityIcons capabilities={model.capabilities} />,
        })),
      ),
    [groupedModels, isInteractionDisabled],
  );

  const selectedModelDetail = useMemo(() => {
    if (!caps.currentModel) return null;
    const data = capsQuery.data;
    if (!data?.chat_models) return null;
    const modelData = data.chat_models[caps.currentModel] as
      | {
          n_ctx: number;
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

  const [localThinkingBudget, setLocalThinkingBudget] = useState<number | null>(
    null,
  );
  const [localMaxTokens, setLocalMaxTokens] = useState<number | null>(null);
  const displayThinkingBudget = localThinkingBudget ?? threadThinkingBudget;
  const displayMaxTokens = localMaxTokens ?? threadMaxTokens;
  const maxOutputTokens = Math.max(
    selectedModelDetail?.maxOutputTokens ?? 16384,
    MIN_OUTPUT_TOKENS,
  );
  const defaultMaxTokens = selectedModelDetail?.defaultMaxTokens ?? 4096;
  const effectiveMaxTokens = displayMaxTokens ?? defaultMaxTokens;
  const clampedMaxTokens = Math.min(
    Math.max(effectiveMaxTokens, MIN_OUTPUT_TOKENS),
    maxOutputTokens,
  );

  const isStartedChat = messages.length > 0;

  useEffect(() => {
    setLocalThinkingBudget(null);
    setLocalMaxTokens(null);
  }, [chatId]);

  useEffect(() => {
    if (!isOpen) {
      setLocalThinkingBudget(null);
      setLocalMaxTokens(null);
    }
  }, [isOpen]);

  // Handlers
  const handleModelSelect = useCallback(
    (modelValue: string) => {
      if (!modelValue) return;
      caps.setCapModel(modelValue);
    },
    [caps],
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
          <Icon icon={ChevronDown} size="sm" tone="muted" />
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
      {!compact && maxTokens > 0 && (
        <>
          <Text size="1" color="gray">
            ·
          </Text>
          <Text size="1" color="gray">
            {formatTokens(maxTokens)}
          </Text>
        </>
      )}
      {!compact && supportsBoostReasoning && isBoostReasoningEnabled && (
        <>
          <Text size="1" color="gray">
            ·
          </Text>
          <Text size="1">
            <ReasoningIcon />
          </Text>
        </>
      )}
      <Icon
        icon={ChevronDown}
        className={styles.chevron}
        size="sm"
        tone="muted"
      />
    </Flex>
  );

  return (
    <KitPopover open={isOpen} onOpenChange={handleOpenChange}>
      <KitPopover.Trigger asChild>
        <button
          className={classNames(
            styles.trigger,
            compact && styles.compactTrigger,
            isInteractionDisabled && styles.disabled,
          )}
          disabled={isInteractionDisabled}
          type="button"
        >
          {triggerContent}
        </button>
      </KitPopover.Trigger>

      <KitPopover.Content
        align="end"
        className={styles.content}
        maxHeight="min(640px, calc(100dvh - 2 * var(--rf-space-5)))"
        maxWidth="min(440px, calc(100vw - var(--rf-space-4)))"
        scrollable={false}
        side="top"
        sideOffset={8}
      >
        <div className={styles.settingsLayout}>
          <div className={`${styles.section} ${styles.modelSection}`}>
            <KitModelSelector
              disabled={isInteractionDisabled}
              groups={modelSelectorGroups}
              models={modelSelectorOptions}
              value={caps.currentModel}
              variant="inline"
              onSelect={handleModelSelect}
            />
          </div>

          <div className={styles.settingsFooter}>
            <Separator size="4" />

            {selectedModelDetail && (
              <>
                <div className={styles.section}>
                  <div className={styles.settingsRow}>
                    <Text
                      size="1"
                      color="gray"
                      weight="medium"
                      className={styles.sectionHeader}
                    >
                      Max tokens
                    </Text>
                    <Text size="1" weight="medium">
                      {displayMaxTokens ?? `${defaultMaxTokens} (default)`}
                    </Text>
                  </div>
                  <div
                    className={classNames(
                      styles.sliderContainer,
                      styles.sliderTrack,
                      threadMaxTokens != null && styles.sliderTrackWithReset,
                    )}
                  >
                    <Text size="1" color="gray">
                      1K
                    </Text>
                    <Slider
                      min={MIN_OUTPUT_TOKENS}
                      max={maxOutputTokens}
                      step={MIN_OUTPUT_TOKENS}
                      value={[clampedMaxTokens]}
                      onValueChange={(values) => setLocalMaxTokens(values[0])}
                      onValueCommit={(values) => {
                        dispatch(setMaxTokens({ chatId, value: values[0] }));
                        setLocalMaxTokens(null);
                      }}
                      disabled={isInteractionDisabled}
                      className={styles.slider}
                    />
                    <Text size="1" color="gray">
                      {formatTokens(maxOutputTokens)}
                    </Text>
                    {threadMaxTokens != null && (
                      <button
                        type="button"
                        className={styles.resetButton}
                        onClick={handleMaxTokensReset}
                        disabled={isInteractionDisabled}
                        aria-label="Reset max tokens"
                      >
                        <Cross1Icon />
                      </button>
                    )}
                  </div>
                </div>
                {supportsBoostReasoning && <Separator size="4" />}
              </>
            )}

            {supportsBoostReasoning && (
              <div className={styles.section}>
                <div className={styles.settingsRow}>
                  <div className={styles.reasoningLabel}>
                    <Text size="1">
                      <ReasoningIcon />
                    </Text>
                    <Text size="1" weight="medium">
                      Reasoning
                    </Text>
                  </div>
                  <Switch
                    checked={isBoostReasoningEnabled}
                    onCheckedChange={handleThinkingToggle}
                    disabled={thinkingDisabled}
                    className="rf-pressable"
                  />
                </div>

                {isStartedChat && (
                  <div className={styles.reasoningWarning}>
                    Changing reasoning mid-chat may break prompt caching (if
                    enabled) and make the next turn much more expensive.
                  </div>
                )}

                {isBoostReasoningEnabled && selectedModelDetail && (
                  <>
                    {selectedModelDetail.reasoningEffortOptions &&
                      selectedModelDetail.reasoningEffortOptions.length > 0 && (
                        <div className={styles.effortRow}>
                          <Text size="1" color="gray">
                            Effort
                          </Text>
                          <div className={styles.effortOptions}>
                            {selectedModelDetail.reasoningEffortOptions.map(
                              (level) => (
                                <button
                                  key={level}
                                  type="button"
                                  className={`${styles.effortButton} ${
                                    (threadReasoningEffort ?? "medium") ===
                                    level
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
                          </div>
                        </div>
                      )}
                    {selectedModelDetail.supportsThinkingBudget && (
                      <div className={styles.budgetSection}>
                        <div className={styles.settingsRow}>
                          <Text size="1" color="gray">
                            Thinking tokens
                          </Text>
                          <Text size="1" weight="medium">
                            {displayThinkingBudget ?? 16384}
                          </Text>
                        </div>
                        <div className={styles.sliderTrack}>
                          <Text size="1" color="gray">
                            1K
                          </Text>
                          <Slider
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
                        </div>
                      </div>
                    )}
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </KitPopover.Content>
    </KitPopover>
  );
};

ChatSettingsDropdown.displayName = "ChatSettingsDropdown";
