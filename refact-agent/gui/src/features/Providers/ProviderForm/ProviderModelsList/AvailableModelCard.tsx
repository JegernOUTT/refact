import {
  type FC,
  type MouseEvent,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import classNames from "classnames";
import {
  Badge,
  Card,
  Flex,
  IconButton,
  Button,
  Switch,
  Text,
  Tooltip,
} from "@radix-ui/themes";
import { Pencil1Icon, TrashIcon } from "@radix-ui/react-icons";
import * as RadixCollapsible from "@radix-ui/react-collapsible";

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
  useToggleModelMutation,
  useSetModelProviderMutation,
  useRemoveCustomModelMutation,
  useGetOpenRouterModelEndpointsQuery,
} from "../../../../services/refact";

import styles from "./ModelCard.module.css";

export type AvailableModelCardProps = {
  model: AvailableModel;
  providerName: string;
  baseProvider: string;
  isReadonlyProvider: boolean;
  onEditModel?: (model: AvailableModel) => void;
};

/**
 * Card component that displays an available model with enable/disable toggle
 */
export const AvailableModelCard: FC<AvailableModelCardProps> = ({
  model,
  providerName,
  baseProvider,
  isReadonlyProvider,
  onEditModel,
}) => {
  const [toggleModel, { isLoading: isToggling }] = useToggleModelMutation();
  const [setModelProvider, { isLoading: isSettingProvider }] =
    useSetModelProviderMutation();
  const [removeCustomModel, { isLoading: isRemoving }] =
    useRemoveCustomModelMutation();
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
    return [...model.provider_variants].sort((a, b) =>
      a.id.localeCompare(b.id),
    );
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
    { providerName, modelId: model.id, useInstanceRoute: false },
    { skip: !shouldFetchEndpoints },
  );

  const resolvedProviderVariants =
    providerVariants.length > 0
      ? providerVariants
      : endpointsData?.provider_variants ?? [];
  const resolvedAvailableProviders =
    availableProviders.length > 0
      ? availableProviders
      : endpointsData?.available_providers ?? [];

  const hasProviderRouting =
    baseProvider === "openrouter" ||
    resolvedProviderVariants.length > 0 ||
    resolvedAvailableProviders.length > 0 ||
    Boolean(model.selected_provider);

  const handleToggle = useCallback(
    async (checked: boolean) => {
      setOptimisticEnabled(checked);
      try {
        await toggleModel({
          providerName,
          modelId: model.id,
          enabled: checked,
        }).unwrap();
      } catch {
        // Revert on error
        setOptimisticEnabled(!checked);
      }
    },
    [toggleModel, providerName, model.id],
  );

  const handleRemove = useCallback(async () => {
    if (!model.is_custom) return;
    try {
      await removeCustomModel({
        providerName,
        modelId: model.id,
      }).unwrap();
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error("Failed to remove custom model:", e);
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
            await toggleModel({
              providerName,
              modelId: model.id,
              enabled: true,
            }).unwrap();
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

  // Format context size for display
  const formatContextSize = (n_ctx: number) => {
    if (n_ctx >= 1000000) return `${(n_ctx / 1000000).toFixed(1)}M`;
    if (n_ctx >= 1000) return `${Math.round(n_ctx / 1000)}K`;
    return `${n_ctx}`;
  };

  const formatPrice = (price?: number | null) =>
    typeof price === "number" ? `$${price.toFixed(2)}` : "–";

  const renderProviderRow = (
    variant: (typeof resolvedProviderVariants)[number],
  ) => {
    const isSelected = optimisticSelectedProvider === variant.id;
    return (
      <div
        key={variant.id}
        className={classNames(styles.providerRow, {
          [styles.providerRowSelected]: isSelected,
        })}
      >
        <Text size="1" className={styles.providerCellPrimary}>
          {variant.tag ?? variant.name ?? variant.id}
        </Text>
        <Text size="1">
          {variant.context_length
            ? formatContextSize(variant.context_length)
            : "–"}
        </Text>
        <Text size="1">
          {variant.max_output_tokens
            ? formatContextSize(variant.max_output_tokens)
            : "–"}
        </Text>
        <Text size="1">{formatPrice(variant.pricing?.prompt)}</Text>
        <Text size="1">{formatPrice(variant.pricing?.generated)}</Text>
        <Text size="1">
          {formatPrice(variant.pricing?.cache_read)} /{" "}
          {formatPrice(variant.pricing?.cache_creation)}
        </Text>
        <Text size="1">
          {typeof variant.latency_last_30m === "number"
            ? `${variant.latency_last_30m.toFixed(2)}s`
            : "–"}
        </Text>
        <Text size="1">
          {typeof variant.throughput_last_30m === "number"
            ? `${variant.throughput_last_30m.toFixed(0)} tps`
            : "–"}
        </Text>
        <Text size="1">
          {typeof variant.uptime_last_30m === "number"
            ? `${variant.uptime_last_30m.toFixed(0)}%`
            : "–"}
        </Text>
        <Text size="1" className={styles.providerCellCaps}>
          {variant.supported_parameters?.length
            ? variant.supported_parameters.join(", ")
            : "–"}
        </Text>
        <Button
          size="1"
          variant={isSelected ? "solid" : "soft"}
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

  return (
    <Card
      className={classNames({ [styles.disabledCard]: isLoading })}
      onClick={handleCardClick}
      style={{ cursor: hasProviderRouting ? "pointer" : "default" }}
    >
      <Flex align="center" justify="between" gap="3">
        <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 0 }}>
          <Flex gap="2" align="center" wrap="wrap">
            <Text
              as="span"
              size="2"
              weight="medium"
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {model.display_name ?? model.id}
            </Text>
            {model.is_custom && (
              <Badge size="1" color="purple">
                Custom
              </Badge>
            )}
          </Flex>

          <Flex gap="2" align="center" wrap="wrap">
            <Tooltip
              content={`Context window: ${model.n_ctx.toLocaleString()} tokens`}
            >
              <ModelDetailIcon icon={<ContextWindowIcon />}>
                {formatContextSize(model.n_ctx)}
              </ModelDetailIcon>
            </Tooltip>
            {model.supports_tools && (
              <Tooltip content="Supports tool/function calling">
                <ModelDetailIcon icon={<ToolsIcon />} />
              </Tooltip>
            )}
            {model.supports_multimodality && (
              <Tooltip content="Supports images/vision">
                <ModelDetailIcon icon={<VisionIcon />} />
              </Tooltip>
            )}
            {(!!model.reasoning_effort_options?.length ||
              !!model.supports_thinking_budget ||
              !!model.supports_adaptive_thinking_budget) && (
              <Tooltip content="Supports reasoning">
                <ModelDetailIcon icon={<ReasoningIcon />} tone="accent" />
              </Tooltip>
            )}
            {typeof model.max_output_tokens === "number" &&
              model.max_output_tokens > 0 && (
                <Tooltip
                  content={`Max output tokens: ${model.max_output_tokens.toLocaleString()}`}
                >
                  <ModelDetailIcon icon={<MaxOutputIcon />}>
                    {formatContextSize(model.max_output_tokens)} out
                  </ModelDetailIcon>
                </Tooltip>
              )}
            {model.pricing && (
              <Tooltip content="Pricing per 1M tokens (input/output)">
                <ModelDetailIcon icon={<PricingIcon />}>
                  ${model.pricing.prompt.toFixed(2)}/$
                  {model.pricing.generated.toFixed(2)}
                </ModelDetailIcon>
              </Tooltip>
            )}
          </Flex>

          {hasProviderRouting && (
            <RadixCollapsible.Root
              open={detailsOpen}
              onOpenChange={setDetailsOpen}
            >
              <RadixCollapsible.Content className={styles.providerPanel}>
                <Text as="span" size="1" color="gray">
                  Selecting a provider will enable the model automatically.
                </Text>
                {resolvedProviderVariants.length > 0 ? (
                  <div className={styles.providerTableWrap}>
                    <div className={styles.providerHeaderRow}>
                      <Text size="1">Provider</Text>
                      <Text size="1">Context</Text>
                      <Text size="1">Max out</Text>
                      <Text size="1">Input</Text>
                      <Text size="1">Output</Text>
                      <Text size="1">Cache R/W</Text>
                      <Text size="1">Latency</Text>
                      <Text size="1">Throughput</Text>
                      <Text size="1">Uptime</Text>
                      <Text size="1">Capabilities</Text>
                      <Text size="1">Action</Text>
                    </div>
                    <div
                      className={classNames(styles.providerRow, {
                        [styles.providerRowSelected]:
                          optimisticSelectedProvider === "",
                      })}
                    >
                      <Text size="1" className={styles.providerCellPrimary}>
                        Auto
                      </Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Text size="1">–</Text>
                      <Button
                        size="1"
                        variant={
                          optimisticSelectedProvider === "" ? "solid" : "soft"
                        }
                        disabled={
                          optimisticSelectedProvider === "" ||
                          isReadonlyProvider ||
                          isLoading
                        }
                        onClick={(event) => {
                          event.stopPropagation();
                          void handleProviderSelect("");
                        }}
                      >
                        {optimisticSelectedProvider === ""
                          ? "Selected"
                          : "Select"}
                      </Button>
                    </div>
                    {resolvedProviderVariants.map(renderProviderRow)}
                  </div>
                ) : (
                  <div className={styles.providerTableWrap}>
                    <Flex direction="column" gap="2">
                      <Flex align="center" justify="between" gap="2">
                        <Text size="1" className={styles.providerCellPrimary}>
                          Auto
                        </Text>
                        <Button
                          size="1"
                          variant={
                            optimisticSelectedProvider === "" ? "solid" : "soft"
                          }
                          disabled={
                            optimisticSelectedProvider === "" ||
                            isReadonlyProvider ||
                            isLoading
                          }
                          onClick={(event) => {
                            event.stopPropagation();
                            void handleProviderSelect("");
                          }}
                        >
                          {optimisticSelectedProvider === ""
                            ? "Selected"
                            : "Select"}
                        </Button>
                      </Flex>
                      {resolvedAvailableProviders.length === 0 && (
                        <Text size="1" color="gray">
                          No provider routing data available.
                        </Text>
                      )}
                      {resolvedAvailableProviders.map((provider) => {
                        const isSelected =
                          optimisticSelectedProvider === provider;
                        return (
                          <Flex
                            key={provider}
                            align="center"
                            justify="between"
                            gap="2"
                          >
                            <Text
                              size="1"
                              className={styles.providerCellPrimary}
                            >
                              {provider}
                            </Text>
                            <Button
                              size="1"
                              variant={isSelected ? "solid" : "soft"}
                              disabled={
                                isSelected || isReadonlyProvider || isLoading
                              }
                              onClick={(event) => {
                                event.stopPropagation();
                                void handleProviderSelect(provider);
                              }}
                            >
                              {isSelected ? "Selected" : "Select"}
                            </Button>
                          </Flex>
                        );
                      })}
                    </Flex>
                  </div>
                )}
              </RadixCollapsible.Content>
            </RadixCollapsible.Root>
          )}
        </Flex>

        <Flex align="center" gap="2">
          {!isReadonlyProvider && (
            <Tooltip
              content={
                model.is_custom
                  ? "Edit custom model"
                  : "Edit model capabilities"
              }
            >
              <IconButton
                size="1"
                variant="ghost"
                color="gray"
                onClick={handleEdit}
                disabled={isLoading}
              >
                <Pencil1Icon />
              </IconButton>
            </Tooltip>
          )}
          {model.is_custom && !isReadonlyProvider && (
            <Tooltip content="Remove custom model">
              <IconButton
                size="1"
                variant="ghost"
                color="red"
                onClick={(event) => {
                  event.stopPropagation();
                  void handleRemove();
                }}
                disabled={isLoading}
              >
                <TrashIcon />
              </IconButton>
            </Tooltip>
          )}
          <Switch
            size="1"
            checked={optimisticEnabled}
            disabled={isReadonlyProvider || isLoading}
            onClick={(event) => event.stopPropagation()}
            onCheckedChange={(checked) => void handleToggle(checked)}
          />
        </Flex>
      </Flex>
    </Card>
  );
};
