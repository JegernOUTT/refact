import { Card, Flex, HoverCard, Text } from "../LongTailPrimitives";
import { Popover } from "../ui";
import {
  ComposerTrigger,
  KitHeader,
  KitKeyValue,
  KitPanel,
  KitProgress,
  KitSection,
  KitTabs,
  kit,
} from "../ComposerKit";
import classNames from "classnames";
import React, { useMemo, useState } from "react";

import {
  calculateUsageInputTokens,
  getCacheCreationTokens,
  getCacheReadTokens,
} from "../../utils/calculateUsageInputTokens";
import { ScrollArea } from "../ScrollArea";
import { useUsageCounter } from "./useUsageCounter";

import {
  selectThreadCurrentMessageTokensById,
  selectThreadImagesById,
  selectEffectiveMaxContextTokensById,
  useThreadId,
} from "../../features/Chat/Thread";
import { TokensMapContent } from "./TokensMapContent";
import { useTokenMap } from "./useTokenMap";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useEffectOnce } from "../../hooks/useEffectOnce";
import {
  useTotalTokenMeteringForChat,
  useTotalUsdForChat,
} from "../../hooks/useTotalCostForChat";
import { formatUsd } from "../../utils/getMetering";

import styles from "./UsageCounter.module.css";

type CircularProgressProps = {
  value: number;
  max: number;
  size?: number;
  strokeWidth?: number;
};

const CircularProgress: React.FC<CircularProgressProps> = ({
  value,
  max,
  size = 20,
  strokeWidth = 3,
}) => {
  const percentage = max > 0 ? Math.min((value / max) * 100, 100) : 0;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = circumference - (percentage / 100) * circumference;

  const isWarning = percentage >= 70 && percentage < 90;
  const isOverflown = percentage >= 90;

  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className={styles.circularProgress}
    >
      <circle
        className={styles.circularProgressBg}
        cx={size / 2}
        cy={size / 2}
        r={radius}
        strokeWidth={strokeWidth}
      />
      <circle
        className={
          isOverflown
            ? styles.circularProgressFillOverflown
            : isWarning
              ? styles.circularProgressFillWarning
              : styles.circularProgressFill
        }
        cx={size / 2}
        cy={size / 2}
        r={radius}
        strokeWidth={strokeWidth}
        strokeDasharray={circumference}
        strokeDashoffset={strokeDashoffset}
        strokeLinecap="round"
      />
    </svg>
  );
};

type UsageCounterProps = { onOpenChange?: (open: boolean) => void } & (
  | {
      isInline?: boolean;
      isMessageEmpty?: boolean;
    }
  | {
      isInline: true;
      isMessageEmpty: boolean;
    }
);

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

const InlineHoverCard: React.FC<{ messageTokens: number }> = ({
  messageTokens,
}) => {
  const chatId = useThreadId();
  const maximumThreadContextTokens = useAppSelector((state) =>
    selectEffectiveMaxContextTokensById(state, chatId),
  );

  return (
    <Flex direction="column" align="start" gap="2">
      {/* TODO: upsale logic might be implemented here to extend maximum context size */}
      {maximumThreadContextTokens && (
        <TokenDisplay
          label="Thread maximum context tokens amount"
          value={maximumThreadContextTokens}
        />
      )}
      <TokenDisplay
        label="Potential tokens amount for current message"
        value={messageTokens}
      />
    </Flex>
  );
};

const InlineHoverTriggerContent: React.FC<{ messageTokens: number }> = ({
  messageTokens,
}) => {
  return (
    <Flex align="center" gap="6px">
      <Text size="1" color="gray" wrap="nowrap">
        {formatNumberToFixed(messageTokens)}{" "}
        {messageTokens === 1 ? "token" : "tokens"}
      </Text>
    </Flex>
  );
};

function contextTone(percentage: number): "warning" | "danger" | undefined {
  if (percentage >= 90) return "danger";
  if (percentage >= 70) return "warning";
  return undefined;
}

const ContextSummary: React.FC<{
  currentSessionTokens: number;
  maxContextTokens: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens?: number;
  cacheCreationTokens?: number;
  totalUsd?: number;
  usdPrompt?: number;
  usdGenerated?: number;
  usdCacheRead?: number;
  usdCacheCreation?: number;
}> = ({
  currentSessionTokens,
  maxContextTokens,
  inputTokens,
  outputTokens,
  cacheReadTokens = 0,
  cacheCreationTokens = 0,
  totalUsd,
  usdPrompt,
  usdGenerated,
  usdCacheRead,
  usdCacheCreation,
}) => {
  const usdRows = [
    ["Prompt", usdPrompt],
    ["Completion", usdGenerated],
    ["Cache read", usdCacheRead],
    ["Cache creation", usdCacheCreation],
  ] as const;
  return (
    <>
      <KitKeyValue
        label="Current"
        value={formatNumberToFixed(currentSessionTokens)}
      />
      <KitKeyValue
        label="Maximum"
        value={formatNumberToFixed(maxContextTokens)}
      />
      {inputTokens > 0 || outputTokens > 0 ? (
        <>
          <KitSection>Total tokens</KitSection>
          {inputTokens > 0 ? (
            <KitKeyValue
              label="Input"
              value={formatNumberToFixed(inputTokens)}
            />
          ) : null}
          {cacheReadTokens > 0 ? (
            <KitKeyValue
              label="Cache read"
              value={formatNumberToFixed(cacheReadTokens)}
            />
          ) : null}
          {cacheCreationTokens > 0 ? (
            <KitKeyValue
              label="Cache creation"
              value={formatNumberToFixed(cacheCreationTokens)}
            />
          ) : null}
          {outputTokens > 0 ? (
            <KitKeyValue
              label="Output"
              value={formatNumberToFixed(outputTokens)}
            />
          ) : null}
        </>
      ) : null}
      {totalUsd !== undefined && totalUsd > 0 ? (
        <>
          <KitSection>Cost</KitSection>
          <KitKeyValue label="Total" value={formatUsd(totalUsd)} />
          {usdRows.map(([label, value]) =>
            value !== undefined && value > 0 ? (
              <KitKeyValue key={label} label={label} value={formatUsd(value)} />
            ) : null,
          )}
        </>
      ) : null}
    </>
  );
};

const DefaultHoverTriggerContent: React.FC<{
  currentSessionTokens: number;
  maxContextTokens: number;
  totalUsd?: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens?: number;
  cacheCreationTokens?: number;
  usdPrompt?: number;
  usdGenerated?: number;
  usdCacheRead?: number;
  usdCacheCreation?: number;
  tokenMap?: import("../../services/refact/chat").TokenMap | null;
  onOpenChange?: (open: boolean) => void;
}> = ({ tokenMap, onOpenChange, ...summary }) => {
  const [tab, setTab] = useState("summary");
  const { currentSessionTokens, maxContextTokens } = summary;
  const percentage =
    maxContextTokens > 0
      ? Math.round((currentSessionTokens / maxContextTokens) * 100)
      : 0;

  return (
    <Popover responsive={false} onOpenChange={onOpenChange}>
      <Popover.Trigger asChild>
        <ComposerTrigger aria-label="Context usage details">
          <CircularProgress
            value={maxContextTokens > 0 ? currentSessionTokens : 0}
            max={maxContextTokens > 0 ? maxContextTokens : 1}
            size={18}
            strokeWidth={2.5}
          />
          <span className={styles.contextTokenLabel}>
            {formatNumberToFixed(currentSessionTokens)}
          </span>
        </ComposerTrigger>
      </Popover.Trigger>
      <Popover.Content
        className={kit.popover}
        side="top"
        align="end"
        maxWidth="300px"
        maxHeight="min(520px, calc(100dvh - var(--rf-space-5)))"
      >
        <KitPanel>
          <KitHeader title="Context usage" meta={`${percentage}%`} metaStrong />
          <KitProgress
            value={percentage}
            tone={contextTone(percentage)}
            label="Context usage"
          />
          <KitTabs
            value={tab}
            onValueChange={setTab}
            label="Context details"
            tabs={[
              {
                value: "summary",
                label: "Summary",
                content: <ContextSummary {...summary} />,
              },
              {
                value: "map",
                label: "Breakdown",
                content: <TokensMapContent tokenMap={tokenMap} />,
              },
            ]}
          />
        </KitPanel>
      </Popover.Content>
    </Popover>
  );
};

export const UsageCounter: React.FC<UsageCounterProps> = ({
  isInline = false,
  isMessageEmpty,
  onOpenChange,
}) => {
  const [open, setOpen] = useState(false);
  const chatId = useThreadId();
  const maybeAttachedImages = useAppSelector((state) =>
    selectThreadImagesById(state, chatId),
  );
  const { totalThreadUsage, isOverflown, isWarning, currentSessionTokens } =
    useUsageCounter();
  const currentMessageTokens = useAppSelector((state) =>
    selectThreadCurrentMessageTokensById(state, chatId),
  );
  const meteringTokens = useTotalTokenMeteringForChat();
  const usdCost = useTotalUsdForChat();
  const tokenMap = useTokenMap();

  const messageTokens = useMemo(() => {
    if (isMessageEmpty && maybeAttachedImages.length === 0) return 0;
    if (!currentMessageTokens) return 0;
    return currentMessageTokens;
  }, [currentMessageTokens, maybeAttachedImages, isMessageEmpty]);

  const inputMeteringTokens = useMemo(() => {
    if (meteringTokens === null) return null;
    return (
      meteringTokens.metering_cache_creation_tokens_n +
      meteringTokens.metering_cache_read_tokens_n +
      meteringTokens.metering_prompt_tokens_n
    );
  }, [meteringTokens]);

  const outputMeteringTokens = useMemo(() => {
    if (meteringTokens === null) return null;
    return meteringTokens.metering_generated_tokens_n;
  }, [meteringTokens]);

  const inputUsageTokens = calculateUsageInputTokens({
    usage: totalThreadUsage,
    keys: [
      "prompt_tokens",
      "cache_creation_input_tokens",
      "cache_read_input_tokens",
    ],
  });
  const outputUsageTokens = calculateUsageInputTokens({
    usage: totalThreadUsage,
    keys: ["completion_tokens"],
  });

  const inputTokens = useMemo(() => {
    return inputMeteringTokens ?? inputUsageTokens;
  }, [inputMeteringTokens, inputUsageTokens]);
  const outputTokens = useMemo(() => {
    return outputMeteringTokens ?? outputUsageTokens;
  }, [outputMeteringTokens, outputUsageTokens]);

  const cacheReadTokens = useMemo(() => {
    const meteringValue = meteringTokens?.metering_cache_read_tokens_n;
    if (typeof meteringValue === "number") {
      return meteringValue;
    }
    return getCacheReadTokens(totalThreadUsage);
  }, [meteringTokens, totalThreadUsage]);

  const cacheCreationTokens = useMemo(() => {
    const meteringValue = meteringTokens?.metering_cache_creation_tokens_n;
    if (typeof meteringValue === "number") {
      return meteringValue;
    }
    return getCacheCreationTokens(totalThreadUsage);
  }, [meteringTokens, totalThreadUsage]);

  const maxContextTokens =
    useAppSelector((state) =>
      selectEffectiveMaxContextTokensById(state, chatId),
    ) ?? 0;

  const shouldUsageBeHidden = useMemo(() => {
    if (isInline) return false;
    return false;
  }, [isInline]);

  useEffectOnce(() => {
    const handleScroll = (event: WheelEvent) => {
      // Checking if the event target is not in the ChatContent
      const chatContent = document.querySelector(
        "[data-element='ChatContent']",
      );
      if (chatContent && chatContent.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("wheel", handleScroll);
    return () => {
      window.removeEventListener("wheel", handleScroll);
    };
  });

  if (shouldUsageBeHidden) return null;

  // For non-inline (panel) usage, render borderless with individual hovercards
  if (!isInline) {
    return (
      <Flex
        align="center"
        className={classNames(
          styles.usageCounterContainer,
          styles.usageCounterBorderless,
          {
            [styles.isWarning]: isWarning,
            [styles.isOverflown]: isOverflown,
          },
        )}
      >
        <DefaultHoverTriggerContent
          currentSessionTokens={currentSessionTokens}
          maxContextTokens={maxContextTokens}
          totalUsd={usdCost?.total_usd}
          inputTokens={inputTokens}
          outputTokens={outputTokens}
          cacheReadTokens={cacheReadTokens}
          cacheCreationTokens={cacheCreationTokens}
          usdPrompt={usdCost?.prompt_usd}
          usdGenerated={usdCost?.generated_usd}
          usdCacheRead={usdCost?.cache_read_usd}
          usdCacheCreation={usdCost?.cache_creation_usd}
          tokenMap={tokenMap}
          onOpenChange={onOpenChange}
        />
      </Flex>
    );
  }

  // For inline usage (chat form), keep the HoverCard with detailed info
  return (
    <HoverCard.Root open={open} onOpenChange={setOpen}>
      <HoverCard.Trigger asChild>
        <Card
          className={classNames(styles.usageCounterContainer, {
            [styles.usageCounterContainerInline]: isInline,
            [styles.isWarning]: isWarning,
            [styles.isOverflown]: isOverflown,
          })}
        >
          <InlineHoverTriggerContent messageTokens={messageTokens} />
        </Card>
      </HoverCard.Trigger>
      <ScrollArea scrollbars="both" asChild>
        <HoverCard.Content
          size="1"
          maxHeight="50vh"
          maxWidth="90vw"
          minWidth="300px"
          avoidCollisions
          align="center"
          side="top"
          hideWhenDetached
        >
          <InlineHoverCard messageTokens={messageTokens} />
        </HoverCard.Content>
      </ScrollArea>
    </HoverCard.Root>
  );
};
