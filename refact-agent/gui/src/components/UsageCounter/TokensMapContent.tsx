import { Box, Flex, Text, ScrollArea, HoverCard } from "../LongTailPrimitives";
import { InfoCircledIcon } from "@radix-ui/react-icons";
import React, { useMemo } from "react";
import type { TokenMap, TokenMapSegment } from "../../services/refact/chat";
import { formatNumberToFixed } from "../../utils/formatNumberToFixed";
import styles from "./TokensMapContent.module.css";

const CATEGORY_COLORS: Record<string, string> = {
  system: "var(--blue-9)",
  project_context: "var(--indigo-9)",
  memories: "var(--violet-9)",
  tools: "var(--purple-9)",
  context_files: "var(--green-9)",
  user_messages: "var(--orange-9)",
  assistant_messages: "var(--cyan-9)",
  tool_results: "var(--pink-9)",
  free: "var(--gray-6)",
};

type SegmentBarProps = {
  segments: TokenMapSegment[];
  maxTokens: number;
};

const SegmentBar: React.FC<SegmentBarProps> = ({ segments, maxTokens }) => {
  return (
    <Flex className={styles.segmentBar}>
      {segments.map((segment, index) => {
        const width = maxTokens > 0 ? (segment.tokens / maxTokens) * 100 : 0;
        if (width < 0.5) return null;
        return (
          <Box
            key={index}
            className={styles.segment}
            style={{
              width: `${Math.max(width, 1)}%`,
              backgroundColor:
                CATEGORY_COLORS[segment.category] || "var(--gray-7)",
            }}
            title={`${segment.label}: ${formatNumberToFixed(
              segment.tokens,
            )} tokens (${segment.percentage.toFixed(1)}%)`}
          />
        );
      })}
    </Flex>
  );
};

type CategoryRowProps = {
  segment: TokenMapSegment;
};

const CategoryRow: React.FC<CategoryRowProps> = ({ segment }) => {
  return (
    <Flex
      align="center"
      justify="between"
      gap="2"
      className={styles.categoryRow}
    >
      <Flex align="center" gap="2">
        <Box
          className={styles.colorDot}
          style={{
            backgroundColor:
              CATEGORY_COLORS[segment.category] || "var(--gray-7)",
          }}
        />
        <Text size="1">{segment.label}</Text>
      </Flex>
      <Flex align="center" gap="2">
        <Text size="1" color="gray">
          {formatNumberToFixed(segment.tokens)}
        </Text>
        <Text size="1" color="gray" className={styles.percentage}>
          ({segment.percentage.toFixed(1)}%)
        </Text>
      </Flex>
    </Flex>
  );
};

type TokensMapContentProps = {
  tokenMap: TokenMap | null | undefined;
};

export const TokensMapContent: React.FC<TokensMapContentProps> = ({
  tokenMap,
}) => {
  const usedSegments = useMemo(() => {
    if (!tokenMap) return [];
    return tokenMap.segments.filter(
      (s) => s.category !== "free" && s.tokens > 0,
    );
  }, [tokenMap]);

  const freeSegment = useMemo(() => {
    if (!tokenMap) return null;
    return tokenMap.segments.find((s) => s.category === "free");
  }, [tokenMap]);

  const topItems = useMemo(() => {
    if (!tokenMap) return [];
    return tokenMap.top_items.slice(0, 5);
  }, [tokenMap]);

  if (!tokenMap) {
    return (
      <Flex direction="column" align="center" justify="center" p="3">
        <Text size="1" color="gray">
          Token breakdown not available yet
        </Text>
        <Text size="1" color="gray">
          Send a message to see breakdown
        </Text>
      </Flex>
    );
  }

  const usedPercentage =
    tokenMap.max_context_tokens > 0
      ? (
          (tokenMap.total_prompt_tokens / tokenMap.max_context_tokens) *
          100
        ).toFixed(1)
      : "0";

  return (
    <Flex direction="column" gap="2" p="1" className={styles.container}>
      <Flex align="center" justify="between" width="100%">
        <Flex align="center" gap="1">
          <Text size="2" weight="bold">
            Token breakdown
          </Text>
          <HoverCard.Root>
            <HoverCard.Trigger>
              <InfoCircledIcon
                color="var(--gray-9)"
                style={{ cursor: "help" }}
              />
            </HoverCard.Trigger>
            <HoverCard.Content size="1" side="top" style={{ maxWidth: 280 }}>
              <Text as="p" size="1" color="gray">
                Total tokens are accurate (from LLM provider).
                <br />
                <br />
                Category breakdown is estimated: we track token deltas between
                assistant responses and distribute them proportionally by
                message content length.
              </Text>
            </HoverCard.Content>
          </HoverCard.Root>
        </Flex>
        <Text size="1" color="gray">
          {usedPercentage}% used
        </Text>
      </Flex>

      <SegmentBar
        segments={tokenMap.segments}
        maxTokens={tokenMap.max_context_tokens}
      />

      <Box my="1" style={{ borderTop: "1px solid var(--gray-a6)" }} />

      <ScrollArea style={{ maxHeight: "200px" }}>
        <Flex direction="column" gap="1">
          {usedSegments.map((segment, index) => (
            <CategoryRow key={index} segment={segment} />
          ))}
          {freeSegment && freeSegment.tokens > 0 && (
            <CategoryRow segment={freeSegment} />
          )}
        </Flex>

        {topItems.length > 0 && (
          <>
            <Box my="2" style={{ borderTop: "1px solid var(--gray-a6)" }} />
            <Text size="1" weight="bold" color="gray" mb="1">
              Top contributors
            </Text>
            <Flex direction="column" gap="1">
              {topItems.map((item, index) => (
                <Flex key={index} align="center" justify="between" gap="2">
                  <Text size="1" color="gray" className={styles.itemLabel}>
                    {item.label}
                  </Text>
                  <Text size="1" color="gray">
                    {formatNumberToFixed(item.tokens)}
                  </Text>
                </Flex>
              ))}
            </Flex>
          </>
        )}
      </ScrollArea>

      <Flex
        align="center"
        justify="between"
        pt="1"
        style={{ borderTop: "1px solid var(--gray-a6)" }}
      >
        <Text size="1" color="gray">
          Total / Max
        </Text>
        <Text size="1">
          {formatNumberToFixed(tokenMap.total_prompt_tokens)} /{" "}
          {formatNumberToFixed(tokenMap.max_context_tokens)}
        </Text>
      </Flex>
    </Flex>
  );
};
