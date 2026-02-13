import { type FC, useCallback, useState } from "react";
import classNames from "classnames";
import {
  Badge,
  Card,
  Flex,
  IconButton,
  Switch,
  Text,
  Tooltip,
} from "@radix-ui/themes";
import { TrashIcon } from "@radix-ui/react-icons";

import type { AvailableModel } from "../../../../services/refact";
import {
  useToggleModelMutation,
  useRemoveCustomModelMutation,
} from "../../../../services/refact";

import styles from "./ModelCard.module.css";

export type AvailableModelCardProps = {
  model: AvailableModel;
  providerName: string;
  isReadonlyProvider: boolean;
};

/**
 * Card component that displays an available model with enable/disable toggle
 */
export const AvailableModelCard: FC<AvailableModelCardProps> = ({
  model,
  providerName,
  isReadonlyProvider,
}) => {
  const [toggleModel, { isLoading: isToggling }] = useToggleModelMutation();
  const [removeCustomModel, { isLoading: isRemoving }] =
    useRemoveCustomModelMutation();
  const [optimisticEnabled, setOptimisticEnabled] = useState(model.enabled);

  const isLoading = isToggling || isRemoving;

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

  // Format context size for display
  const formatContextSize = (n_ctx: number) => {
    if (n_ctx >= 1000000) return `${(n_ctx / 1000000).toFixed(1)}M`;
    if (n_ctx >= 1000) return `${Math.round(n_ctx / 1000)}K`;
    return `${n_ctx}`;
  };

  return (
    <Card className={classNames({ [styles.disabledCard]: isLoading })}>
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
              <Text as="span" size="1" color="gray">
                📏 {formatContextSize(model.n_ctx)}
              </Text>
            </Tooltip>
            {model.supports_tools && (
              <Tooltip content="Supports tool/function calling">
                <Text as="span" size="1" color="gray">
                  🔧
                </Text>
              </Tooltip>
            )}
            {model.supports_multimodality && (
              <Tooltip content="Supports images/vision">
                <Text as="span" size="1" color="gray">
                  👁️
                </Text>
              </Tooltip>
            )}
            {(model.reasoning_effort_options || model.supports_thinking_budget || model.supports_adaptive_thinking_budget) && (
              <Tooltip content="Supports reasoning">
                <Text as="span" size="1" color="gray">
                  🧠
                </Text>
              </Tooltip>
            )}
          </Flex>
        </Flex>

        <Flex align="center" gap="2">
          {model.is_custom && !isReadonlyProvider && (
            <Tooltip content="Remove custom model">
              <IconButton
                size="1"
                variant="ghost"
                color="red"
                onClick={() => void handleRemove()}
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
            onCheckedChange={(checked) => void handleToggle(checked)}
          />
        </Flex>
      </Flex>
    </Card>
  );
};
