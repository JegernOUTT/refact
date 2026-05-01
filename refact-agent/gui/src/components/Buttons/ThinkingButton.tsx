import React from "react";
import { useThinking } from "../../hooks/useThinking";
import { useAppSelector } from "../../hooks";
import { selectThreadBoostReasoning } from "../../features/Chat";
import { Button, Flex, HoverCard, Skeleton, Text } from "@radix-ui/themes";
import { MagicWandIcon } from "@radix-ui/react-icons";

const ThinkButtonContent: React.FC = () => (
  <Flex as="span" align="center" gap="1">
    <MagicWandIcon />
    Think
  </Flex>
);

export const ThinkingButton: React.FC = () => {
  const isBoostReasoningEnabled = useAppSelector(selectThreadBoostReasoning);
  const {
    handleReasoningChange,
    shouldBeDisabled,
    noteText,
    areCapsInitialized,
    supportsBoostReasoning,
  } = useThinking();
  if (!areCapsInitialized) {
    return (
      <Skeleton>
        <Button size="1">
          <ThinkButtonContent />
        </Button>
      </Skeleton>
    );
  }

  if (!supportsBoostReasoning) {
    return null;
  }

  return (
    <Flex gap="2" align="center">
      <HoverCard.Root>
        <HoverCard.Trigger>
          <Button
            size="1"
            onClick={(event) =>
              handleReasoningChange(event, !isBoostReasoningEnabled)
            }
            variant={isBoostReasoningEnabled ? "solid" : "outline"}
            disabled={shouldBeDisabled}
          >
            <ThinkButtonContent />
          </Button>
        </HoverCard.Trigger>
        <HoverCard.Content
          size="2"
          maxWidth="500px"
          width="calc(100vw - (var(--space-9) * 2.5))"
          side="top"
        >
          <Text as="p" size="2">
            When enabled, the model will use enhanced reasoning capabilities
            which may improve problem-solving for complex tasks.
          </Text>

          {noteText && (
            <Text as="p" color="gray" size="1" mt="1">
              {noteText}
            </Text>
          )}
        </HoverCard.Content>
      </HoverCard.Root>
    </Flex>
  );
};
