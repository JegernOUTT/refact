import React, { useState } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import type { SummarizationMessage as SummarizationMessageType } from "../../services/refact/types";

interface SummarizationMessageProps {
  message: SummarizationMessageType;
}

export const SummarizationMessage: React.FC<SummarizationMessageProps> = ({
  message,
}) => {
  const [open, setOpen] = useState(false);

  const rangeLabel = message.summarized_range
    ? `Messages ${message.summarized_range[0] + 1}–${
        message.summarized_range[1] + 1
      } summarized`
    : "Messages summarized";

  const tierLabel =
    message.summarization_tier === "tier2_reactive"
      ? "Reactive compaction"
      : message.summarization_tier === "tier1_llm"
        ? "LLM"
        : "deterministic";

  const tokenLabel = message.summarized_token_estimate
    ? ` — ~${message.summarized_token_estimate} tokens`
    : "";

  return (
    <Box my="1">
      <Flex
        align="center"
        gap="2"
        px="2"
        py="1"
        style={{
          cursor: "pointer",
          background: "var(--gray-a3)",
          borderRadius: "var(--radius-2)",
          userSelect: "none",
        }}
        onClick={() => setOpen((v) => !v)}
      >
        <Text size="1" color="gray">
          🗜️ {rangeLabel} [{tierLabel}]{tokenLabel}
        </Text>
        <Text size="1" color="gray" ml="auto">
          {open ? "▲" : "▼"}
        </Text>
      </Flex>
      {open && (
        <Box
          px="2"
          py="2"
          style={{
            background: "var(--gray-a2)",
            borderRadius: "0 0 var(--radius-2) var(--radius-2)",
            whiteSpace: "pre-wrap",
          }}
        >
          <Text size="1" color="gray">
            {typeof message.content === "string" ? message.content : ""}
          </Text>
        </Box>
      )}
    </Box>
  );
};
