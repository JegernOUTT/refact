import React, { useCallback } from "react";
import { Flex, Text, HoverCard, Box, Tooltip } from "@radix-ui/themes";
import {
  CopyIcon,
  Share1Icon,
  TrashIcon,
  BarChartIcon,
} from "@radix-ui/react-icons";
import { Usage } from "../../services/refact";
import { Checkpoint } from "../../features/Checkpoints/types";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";
import { calculateUsageInputTokens } from "../../utils/calculateUsageInputTokens";
import { formatUsd } from "../../utils/getMetering";
import { Coin } from "../../images";
import { CheckpointButton } from "../../features/Checkpoints";
import styles from "./MessageFooter.module.css";

type MessageFooterProps = {
  messageId?: string;
  onCopy?: () => void;
  onBranch?: (messageId: string) => void;
  onDelete?: (messageId: string) => void;
  usage?: Usage | null;
  metering_coins_prompt?: number;
  metering_coins_generated?: number;
  metering_coins_cache_creation?: number;
  metering_coins_cache_read?: number;
  // For user messages with checkpoints
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

const CoinDisplay: React.FC<{ label: string; value: number }> = ({
  label,
  value,
}) => (
  <Flex align="center" justify="between" width="100%" gap="4">
    <Text size="1" weight="bold">
      {label}
    </Text>
    <Text size="1">
      <Flex align="center" gap="2">
        {Math.round(value)} <Coin width="12px" height="12px" />
      </Flex>
    </Text>
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
  metering_coins_prompt = 0,
  metering_coins_generated = 0,
  metering_coins_cache_creation = 0,
  metering_coins_cache_read = 0,
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

  const totalCoins =
    metering_coins_prompt +
    metering_coins_generated +
    metering_coins_cache_creation +
    metering_coins_cache_read;

  const meteringUsd = usage?.metering_usd;
  const hasUsd = meteringUsd !== undefined && meteringUsd.total_usd > 0;
  const showCoins = !hasUsd && totalCoins > 0;

  const contextTokens = calculateUsageInputTokens({
    usage,
    keys: [
      "prompt_tokens",
      "cache_creation_input_tokens",
      "cache_read_input_tokens",
    ],
  });
  const hasUsageInfo = Boolean(usage && contextTokens > 0) || showCoins || hasUsd;

  return (
    <div className={styles.footerLane}>
      <div className={styles.footerContent}>
        {/* Checkpoints button (for user messages) */}
        {checkpoints &&
          checkpoints.length > 0 &&
          messageIndex !== undefined && (
            <CheckpointButton
              checkpoints={checkpoints}
              messageIndex={messageIndex}
            />
          )}
        {/* Action buttons - styled like tool card icons */}
        {onCopy && (
          <Tooltip content="Copy message">
            <div className={styles.footerItem} onClick={onCopy}>
              <CopyIcon />
            </div>
          </Tooltip>
        )}
        {onBranch && messageId && (
          <Tooltip content="Branch from here">
            <div className={styles.footerItem} onClick={handleBranch}>
              <Share1Icon />
            </div>
          </Tooltip>
        )}
        {onDelete && messageId && (
          <Tooltip content="Delete message">
            <div
              className={`${styles.footerItem} ${styles.footerItemDanger}`}
              onClick={handleDelete}
            >
              <TrashIcon />
            </div>
          </Tooltip>
        )}

        {/* Token/cost info - styled like tool card icons */}
        {hasUsageInfo && (
          <HoverCard.Root>
            <HoverCard.Trigger>
              <Flex align="center" gap="2">
                {contextTokens > 0 && (
                  <div className={styles.footerItem}>
                    <BarChartIcon />
                    <span>{formatNumberToFixed(contextTokens)}</span>
                  </div>
                )}
                {showCoins && (
                  <div className={styles.footerItem}>
                    <Coin />
                    <span>{Math.round(totalCoins)}</span>
                  </div>
                )}
                {hasUsd && (
                  <div className={styles.footerItem}>
                    <span>{formatUsd(meteringUsd.total_usd)}</span>
                  </div>
                )}
              </Flex>
            </HoverCard.Trigger>
            <HoverCard.Content size="1" maxWidth="300px">
              <Flex direction="column" gap="2">
                <Text size="2" weight="bold" mb="1">
                  This Message
                </Text>

                {usage && (
                  <>
                    <TokenDisplay label="Context size" value={contextTokens} />
                    <TokenDisplay label="Output tokens" value={outputTokens} />
                    {usage.completion_tokens_details?.reasoning_tokens !=
                      null &&
                      usage.completion_tokens_details.reasoning_tokens > 0 && (
                        <TokenDisplay
                          label="Reasoning tokens"
                          value={
                            usage.completion_tokens_details.reasoning_tokens
                          }
                        />
                      )}
                  </>
                )}

                {showCoins && (
                  <>
                    <Box
                      my="2"
                      style={{ borderTop: "1px solid var(--gray-a6)" }}
                    />
                    <Flex align="center" justify="between" width="100%" mb="1">
                      <Text size="2" weight="bold">
                        Cost
                      </Text>
                      <Text size="2">
                        <Flex align="center" gap="2">
                          {Math.round(totalCoins)}{" "}
                          <Coin width="14px" height="14px" />
                        </Flex>
                      </Text>
                    </Flex>
                    {metering_coins_prompt > 0 && (
                      <CoinDisplay
                        label="Prompt"
                        value={metering_coins_prompt}
                      />
                    )}
                    {metering_coins_generated > 0 && (
                      <CoinDisplay
                        label="Completion"
                        value={metering_coins_generated}
                      />
                    )}
                    {metering_coins_cache_read > 0 && (
                      <CoinDisplay
                        label="Cache read"
                        value={metering_coins_cache_read}
                      />
                    )}
                    {metering_coins_cache_creation > 0 && (
                      <CoinDisplay
                        label="Cache creation"
                        value={metering_coins_cache_creation}
                      />
                    )}
                  </>
                )}

                {hasUsd && (
                  <>
                    <Box
                      my="2"
                      style={{ borderTop: "1px solid var(--gray-a6)" }}
                    />
                    <Flex align="center" justify="between" width="100%" mb="1">
                      <Text size="2" weight="bold">
                        Cost
                      </Text>
                      <Text size="2">
                        {formatUsd(meteringUsd.total_usd)}
                      </Text>
                    </Flex>
                    <UsdDisplay label="Prompt" value={meteringUsd.prompt_usd} />
                    <UsdDisplay label="Completion" value={meteringUsd.generated_usd} />
                    {meteringUsd.cache_read_usd !== undefined && meteringUsd.cache_read_usd > 0 && (
                      <UsdDisplay label="Cache read" value={meteringUsd.cache_read_usd} />
                    )}
                    {meteringUsd.cache_creation_usd !== undefined && meteringUsd.cache_creation_usd > 0 && (
                      <UsdDisplay label="Cache creation" value={meteringUsd.cache_creation_usd} />
                    )}
                  </>
                )}
              </Flex>
            </HoverCard.Content>
          </HoverCard.Root>
        )}
      </div>
    </div>
  );
};

// Wrapper component to enable CSS hover on parent
export const MessageWrapper: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  return <div className={styles.messageWrapper}>{children}</div>;
};
