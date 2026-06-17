import React, { useCallback } from "react";
import { Flex, Text } from "@radix-ui/themes";
import * as HoverCard from "@radix-ui/react-hover-card";
import { BarChart3, Copy, GitBranch, Trash2 } from "lucide-react";
import { Usage } from "../../services/refact";
import { Checkpoint } from "../../features/Checkpoints/types";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";
import {
  calculateUsageInputTokens,
  getCacheCreationTokens,
  getCacheReadTokens,
} from "../../utils/calculateUsageInputTokens";
import { formatUsd } from "../../utils/getMetering";
import { CheckpointButton } from "../../features/Checkpoints";
import { Icon, IconButton, Tooltip } from "../ui";
import styles from "./MessageFooter.module.css";

type MessageFooterProps = {
  messageId?: string;
  onCopy?: () => void;
  onBranch?: (messageId: string) => void;
  onDelete?: (messageId: string) => void;
  usage?: Usage | null;
  checkpoints?: Checkpoint[] | null;
  messageIndex?: number;
};

const TokenDisplay: React.FC<{ label: string; value: number }> = ({
  label,
  value,
}) => (
  <Flex align="center" justify="between" width="100%" gap="4">
    <Text size="1" weight="bold">
      {label}
    </Text>
    <Text size="1">{formatNumberToFixed(value)}</Text>
  </Flex>
);

const UsdDisplay: React.FC<{ label: string; value: number | undefined }> = ({
  label,
  value,
}) => (
  <Flex align="center" justify="between" width="100%" gap="4">
    <Text size="1" weight="bold">
      {label}
    </Text>
    <Text size="1">{formatUsd(value)}</Text>
  </Flex>
);

export const MessageFooter: React.FC<MessageFooterProps> = ({
  messageId,
  onCopy,
  onBranch,
  onDelete,
  usage,
  checkpoints,
  messageIndex,
}) => {
  const handleBranch = useCallback(() => {
    if (messageId && onBranch) {
      onBranch(messageId);
    }
  }, [messageId, onBranch]);

  const handleDelete = useCallback(() => {
    if (messageId && onDelete) {
      onDelete(messageId);
    }
  }, [messageId, onDelete]);

  const outputTokens = calculateUsageInputTokens({
    usage,
    keys: ["completion_tokens"],
  });

  const meteringUsd = usage?.metering_usd;
  const hasUsd = meteringUsd !== undefined && meteringUsd.total_usd > 0;

  const contextTokens = calculateUsageInputTokens({
    usage,
    keys: [
      "prompt_tokens",
      "cache_creation_input_tokens",
      "cache_read_input_tokens",
    ],
  });
  const cacheReadTokens = getCacheReadTokens(usage);
  const cacheCreationTokens = getCacheCreationTokens(usage);
  const hasUsageInfo = Boolean(usage && contextTokens > 0) || hasUsd;

  return (
    <div className={styles.footerLane}>
      <div className={styles.footerContent}>
        {checkpoints &&
          checkpoints.length > 0 &&
          messageIndex !== undefined && (
            <CheckpointButton
              checkpoints={checkpoints}
              messageIndex={messageIndex}
            />
          )}
        {onCopy && (
          <Tooltip delayDuration={150}>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Copy message"
                className={styles.footerButton}
                icon={Copy}
                onClick={onCopy}
                size="sm"
                variant="plain"
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Copy message</Tooltip.Content>
          </Tooltip>
        )}
        {onBranch && messageId && (
          <Tooltip delayDuration={150}>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Branch from here"
                className={styles.footerButton}
                icon={GitBranch}
                onClick={handleBranch}
                size="sm"
                variant="plain"
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Branch from here</Tooltip.Content>
          </Tooltip>
        )}
        {onDelete && messageId && (
          <Tooltip delayDuration={150}>
            <Tooltip.Trigger asChild>
              <IconButton
                aria-label="Delete message"
                className={styles.footerDangerButton}
                icon={Trash2}
                onClick={handleDelete}
                size="sm"
                variant="plain"
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Delete message</Tooltip.Content>
          </Tooltip>
        )}

        {hasUsageInfo && (
          <HoverCard.Root openDelay={150} closeDelay={120}>
            <HoverCard.Trigger asChild>
              <button className={styles.usageTrigger} type="button">
                {contextTokens > 0 && (
                  <span className={styles.footerItem}>
                    <Icon icon={BarChart3} size="sm" tone="muted" />
                    <span>{formatNumberToFixed(contextTokens)}</span>
                  </span>
                )}
                {hasUsd && (
                  <span className={styles.footerItem}>
                    <span>{formatUsd(meteringUsd.total_usd)}</span>
                  </span>
                )}
              </button>
            </HoverCard.Trigger>
            <HoverCard.Portal>
              <HoverCard.Content
                align="center"
                className={`${styles.usageContent} rf-popover-motion`}
                collisionPadding={12}
                side="top"
                sideOffset={8}
              >
                <Flex direction="column" gap="2">
                  <Text size="2" weight="bold" mb="1">
                    This Message
                  </Text>

                  {usage && (
                    <>
                      <TokenDisplay
                        label="Context size"
                        value={contextTokens}
                      />
                      {cacheReadTokens > 0 && (
                        <TokenDisplay
                          label="Cache read"
                          value={cacheReadTokens}
                        />
                      )}
                      {cacheCreationTokens > 0 && (
                        <TokenDisplay
                          label="Cache creation"
                          value={cacheCreationTokens}
                        />
                      )}
                      <TokenDisplay
                        label="Output tokens"
                        value={outputTokens}
                      />
                      {usage.completion_tokens_details?.reasoning_tokens !=
                        null &&
                        usage.completion_tokens_details.reasoning_tokens >
                          0 && (
                          <TokenDisplay
                            label="Reasoning tokens"
                            value={
                              usage.completion_tokens_details.reasoning_tokens
                            }
                          />
                        )}
                    </>
                  )}

                  {hasUsd && (
                    <>
                      <div className={styles.usageSeparator} />
                      <Flex
                        align="center"
                        justify="between"
                        width="100%"
                        mb="1"
                      >
                        <Text size="2" weight="bold">
                          Cost
                        </Text>
                        <Text size="2">{formatUsd(meteringUsd.total_usd)}</Text>
                      </Flex>
                      <UsdDisplay
                        label="Prompt"
                        value={meteringUsd.prompt_usd}
                      />
                      <UsdDisplay
                        label="Completion"
                        value={meteringUsd.generated_usd}
                      />
                      {meteringUsd.cache_read_usd !== undefined &&
                        meteringUsd.cache_read_usd > 0 && (
                          <UsdDisplay
                            label="Cache read"
                            value={meteringUsd.cache_read_usd}
                          />
                        )}
                      {meteringUsd.cache_creation_usd !== undefined &&
                        meteringUsd.cache_creation_usd > 0 && (
                          <UsdDisplay
                            label="Cache creation"
                            value={meteringUsd.cache_creation_usd}
                          />
                        )}
                    </>
                  )}
                </Flex>
              </HoverCard.Content>
            </HoverCard.Portal>
          </HoverCard.Root>
        )}
      </div>
    </div>
  );
};

export const MessageWrapper: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  return (
    <div className={`${styles.messageWrapper} rf-enter-rise`}>{children}</div>
  );
};
