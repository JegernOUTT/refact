import { type FC, type MouseEvent, useCallback, useEffect, useMemo, useState } from "react";
import classNames from "classnames";
import { Pencil, Trash2 } from "lucide-react";
import * as RadixCollapsible from "@radix-ui/react-collapsible";

import { Badge, Button, IconButton, Surface, Switch, Tooltip } from "../../../../components/ui";
import {
  ContextWindowIcon,
  MaxOutputIcon,
  ModelDetailIcon,
  PricingIcon,
  ReasoningIcon,
  ToolsIcon,
  VisionIcon,
} from "./components/CapabilityIcons";

import type { AvailableModel } from "../../../../services/refact";
import {
  useGetOpenRouterModelEndpointsQuery,
  useRemoveCustomModelMutation,
  useSetModelProviderMutation,
  useToggleModelMutation,
} from "../../../../services/refact";

import styles from "./ModelCard.module.css";

export type AvailableModelCardProps = {
  model: AvailableModel;
  providerName: string;
  baseProvider: string;
  isReadonlyProvider: boolean;
  onEditModel?: (model: AvailableModel) => void;
};

export const AvailableModelCard: FC<AvailableModelCardProps> = ({
  model,
  providerName,
  baseProvider,
  isReadonlyProvider,
  onEditModel,
}) => {
  const [toggleModel, { isLoading: isToggling }] = useToggleModelMutation();
  const [setModelProvider, { isLoading: isSettingProvider }] = useSetModelProviderMutation();
  const [removeCustomModel, { isLoading: isRemoving }] = useRemoveCustomModelMutation();
  const [optimisticEnabled, setOptimisticEnabled] = useState(model.enabled);
  const [optimisticSelectedProvider, setOptimisticSelectedProvider] = useState(
    model.selected_provider ?? "",
  );
  const [detailsOpen, setDetailsOpen] = useState(false);

  useEffect(() => {
    setOptimisticEnabled(model.enabled);
  }, [model.enabled]);

  useEffect(() => {
    setOptimisticSelectedProvider(model.selected_provider ?? "");
  }, [model.selected_provider]);

  const isLoading = isToggling || isRemoving || isSettingProvider;

  const providerVariants = useMemo(() => {
    if (!model.provider_variants?.length) return [];
    return [...model.provider_variants].sort((a, b) => a.id.localeCompare(b.id));
  }, [model.provider_variants]);

  const availableProviders = useMemo(() => {
    if (!model.available_providers?.length) return [];
    return [...model.available_providers].sort((a, b) => a.localeCompare(b));
  }, [model.available_providers]);

  const shouldFetchEndpoints =
    baseProvider === "openrouter" &&
    detailsOpen &&
    providerVariants.length === 0 &&
    availableProviders.length === 0;

  const { data: endpointsData } = useGetOpenRouterModelEndpointsQuery(
    { providerName, modelId: model.id, useInstanceRoute: true },
    { skip: !shouldFetchEndpoints },
  );

  const resolvedProviderVariants =
    providerVariants.length > 0 ? providerVariants : endpointsData?.provider_variants ?? [];
  const resolvedAvailableProviders =
    availableProviders.length > 0 ? availableProviders : endpointsData?.available_providers ?? [];

  const hasProviderRouting =
    baseProvider === "openrouter" ||
    resolvedProviderVariants.length > 0 ||
    resolvedAvailableProviders.length > 0 ||
    Boolean(model.selected_provider);

  const handleToggle = useCallback(
    async (checked: boolean) => {
      setOptimisticEnabled(checked);
      try {
        await toggleModel({ providerName, modelId: model.id, enabled: checked }).unwrap();
      } catch {
        setOptimisticEnabled(!checked);
      }
    },
    [toggleModel, providerName, model.id],
  );

  const handleRemove = useCallback(async () => {
    if (!model.is_custom) return;
    try {
      await removeCustomModel({ providerName, modelId: model.id }).unwrap();
    } catch {
      return;
    }
  }, [removeCustomModel, providerName, model.id, model.is_custom]);

  const handleEdit = useCallback(
    (event: MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      onEditModel?.(model);
    },
    [model, onEditModel],
  );

  const handleProviderSelect = useCallback(
    async (provider: string) => {
      const normalized = provider === "" ? null : provider;
      const previous = optimisticSelectedProvider;
      setOptimisticSelectedProvider(provider);
      try {
        await setModelProvider({
          providerName,
          modelId: model.id,
          selectedProvider: normalized,
        }).unwrap();
        if (!optimisticEnabled) {
          setOptimisticEnabled(true);
          try {
            await toggleModel({ providerName, modelId: model.id, enabled: true }).unwrap();
          } catch {
            setOptimisticEnabled(false);
          }
        }
      } catch {
        setOptimisticSelectedProvider(previous);
      }
    },
    [
      model.id,
      optimisticEnabled,
      optimisticSelectedProvider,
      providerName,
      setModelProvider,
      toggleModel,
    ],
  );

  const formatContextSize = (n_ctx: number) => {
    if (n_ctx >= 1000000) return `${(n_ctx / 1000000).toFixed(1)}M`;
    if (n_ctx >= 1000) return `${Math.round(n_ctx / 1000)}K`;
    return `${n_ctx}`;
  };

  const formatPrice = (price?: number | null) =>
    typeof price === "number" ? `$${price.toFixed(2)}` : "–";

  const renderProviderRow = (variant: (typeof resolvedProviderVariants)[number]) => {
    const isSelected = optimisticSelectedProvider === variant.id;
    return (
      <div
        key={variant.id}
        className={classNames(styles.providerRow, {
          [styles.providerRowSelected]: isSelected,
        })}
      >
        <span className={styles.providerCellPrimary}>{variant.tag ?? variant.name ?? variant.id}</span>
        <span>{variant.context_length ? formatContextSize(variant.context_length) : "–"}</span>
        <span>{variant.max_output_tokens ? formatContextSize(variant.max_output_tokens) : "–"}</span>
        <span>{formatPrice(variant.pricing?.prompt)}</span>
        <span>{formatPrice(variant.pricing?.generated)}</span>
        <span>
          {formatPrice(variant.pricing?.cache_read)} / {formatPrice(variant.pricing?.cache_creation)}
        </span>
        <span>
          {typeof variant.latency_last_30m === "number"
            ? `${variant.latency_last_30m.toFixed(2)}s`
            : "–"}
        </span>
        <span>
          {typeof variant.throughput_last_30m === "number"
            ? `${variant.throughput_last_30m.toFixed(0)} tps`
            : "–"}
        </span>
        <span>
          {typeof variant.uptime_last_30m === "number"
            ? `${variant.uptime_last_30m.toFixed(0)}%`
            : "–"}
        </span>
        <span className={styles.providerCellCaps}>
          {variant.supported_parameters?.length ? variant.supported_parameters.join(", ") : "–"}
        </span>
        <Button
          size="sm"
          variant={isSelected ? "primary" : "soft"}
          disabled={isSelected || isReadonlyProvider || isLoading}
          onClick={(event) => {
            event.stopPropagation();
            void handleProviderSelect(variant.id);
          }}
        >
          {isSelected ? "Selected" : "Select"}
        </Button>
      </div>
    );
  };

  const handleCardClick = useCallback(() => {
    if (!hasProviderRouting) return;
    setDetailsOpen((prev) => !prev);
  }, [hasProviderRouting]);

  const renderAutoProviderButton = () => {
    const isSelected = optimisticSelectedProvider === "";
    return (
      <Button
        size="sm"
        variant={isSelected ? "primary" : "soft"}
        disabled={isSelected || isReadonlyProvider || isLoading}
        onClick={(event) => {
          event.stopPropagation();
          void handleProviderSelect("");
        }}
      >
        {isSelected ? "Selected" : "Select"}
      </Button>
    );
  };

  return (
    <Surface
      variant="plain"
      className={classNames(styles.modelCard, "rf-enter", {
        [styles.disabledCard]: isLoading,
        [styles.clickable]: hasProviderRouting,
      })}
      onClick={handleCardClick}
    >
      <div className={styles.modelHeader}>
        <div className={styles.modelCopy}>
          <div className={styles.modelTitleRow}>
            <span className={styles.modelName}>{model.display_name ?? model.id}</span>
            {model.is_custom ? <Badge tone="accent">Custom</Badge> : null}
          </div>

          <div className={styles.modelMetaRow}>
            <Tooltip>
              <Tooltip.Trigger asChild>
                <span>
                  <ModelDetailIcon icon={<ContextWindowIcon />}>
                    {formatContextSize(model.n_ctx)}
                  </ModelDetailIcon>
                </span>
              </Tooltip.Trigger>
              <Tooltip.Content>Context window: {model.n_ctx.toLocaleString()} tokens</Tooltip.Content>
            </Tooltip>
            {model.supports_tools ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<ToolsIcon />} />
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>Supports tool/function calling</Tooltip.Content>
              </Tooltip>
            ) : null}
            {model.supports_multimodality ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<VisionIcon />} />
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>Supports images/vision</Tooltip.Content>
              </Tooltip>
            ) : null}
            {(!!model.reasoning_effort_options?.length ||
              (model.supports_thinking_budget ?? false) ||
              (model.supports_adaptive_thinking_budget ?? false)) ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<ReasoningIcon />} tone="accent" />
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>Supports reasoning</Tooltip.Content>
              </Tooltip>
            ) : null}
            {typeof model.max_output_tokens === "number" && model.max_output_tokens > 0 ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<MaxOutputIcon />}>
                      {formatContextSize(model.max_output_tokens)} out
                    </ModelDetailIcon>
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>
                  Max output tokens: {model.max_output_tokens.toLocaleString()}
                </Tooltip.Content>
              </Tooltip>
            ) : null}
            {model.pricing ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<PricingIcon />}>
                      ${model.pricing.prompt.toFixed(2)}/${model.pricing.generated.toFixed(2)}
                    </ModelDetailIcon>
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>Pricing per 1M tokens (input/output)</Tooltip.Content>
              </Tooltip>
            ) : null}
          </div>

          {hasProviderRouting ? (
            <RadixCollapsible.Root open={detailsOpen} onOpenChange={setDetailsOpen}>
              <RadixCollapsible.Content className={styles.providerPanel}>
                <span className={styles.mutedText}>
                  Selecting a provider will enable the model automatically.
                </span>
                {resolvedProviderVariants.length > 0 ? (
                  <div className={styles.providerTableWrap}>
                    <div className={styles.providerHeaderRow}>
                      <span>Provider</span>
                      <span>Context</span>
                      <span>Max out</span>
                      <span>Input</span>
                      <span>Output</span>
                      <span>Cache R/W</span>
                      <span>Latency</span>
                      <span>Throughput</span>
                      <span>Uptime</span>
                      <span>Capabilities</span>
                      <span>Action</span>
                    </div>
                    <div
                      className={classNames(styles.providerRow, {
                        [styles.providerRowSelected]: optimisticSelectedProvider === "",
                      })}
                    >
                      <span className={styles.providerCellPrimary}>Auto</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      <span>–</span>
                      {renderAutoProviderButton()}
                    </div>
                    {resolvedProviderVariants.map(renderProviderRow)}
                  </div>
                ) : (
                  <div className={styles.providerTableWrap}>
                    <div className={styles.availableProvidersList}>
                      <div className={styles.availableProviderRow}>
                        <span className={styles.providerCellPrimary}>Auto</span>
                        {renderAutoProviderButton()}
                      </div>
                      {resolvedAvailableProviders.length === 0 ? (
                        <span className={styles.mutedText}>No provider routing data available.</span>
                      ) : null}
                      {resolvedAvailableProviders.map((provider) => {
                        const isSelected = optimisticSelectedProvider === provider;
                        return (
                          <div className={styles.availableProviderRow} key={provider}>
                            <span className={styles.providerCellPrimary}>{provider}</span>
                            <Button
                              size="sm"
                              variant={isSelected ? "primary" : "soft"}
                              disabled={isSelected || isReadonlyProvider || isLoading}
                              onClick={(event) => {
                                event.stopPropagation();
                                void handleProviderSelect(provider);
                              }}
                            >
                              {isSelected ? "Selected" : "Select"}
                            </Button>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                )}
              </RadixCollapsible.Content>
            </RadixCollapsible.Root>
          ) : null}
        </div>

        <div className={styles.modelActions}>
          {!isReadonlyProvider ? (
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  size="sm"
                  variant="ghost"
                  aria-label={model.is_custom ? "Edit custom model" : "Edit model capabilities"}
                  icon={Pencil}
                  onClick={handleEdit}
                  disabled={isLoading}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>
                {model.is_custom ? "Edit custom model" : "Edit model capabilities"}
              </Tooltip.Content>
            </Tooltip>
          ) : null}
          {model.is_custom && !isReadonlyProvider ? (
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  size="sm"
                  variant="danger"
                  aria-label="Remove custom model"
                  icon={Trash2}
                  onClick={(event) => {
                    event.stopPropagation();
                    void handleRemove();
                  }}
                  disabled={isLoading}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>Remove custom model</Tooltip.Content>
            </Tooltip>
          ) : null}
          <Switch
            checked={optimisticEnabled}
            disabled={isReadonlyProvider || isLoading}
            onClick={(event) => event.stopPropagation()}
            onCheckedChange={(checked) => void handleToggle(checked)}
          />
        </div>
      </div>
    </Surface>
  );
};
