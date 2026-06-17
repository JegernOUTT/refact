import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import { WandSparkles } from "lucide-react";
import { useThinking } from "../../hooks/useThinking";
import { useAppSelector } from "../../hooks";
import {
  selectThreadBoostReasoningById,
  useThreadId,
} from "../../features/Chat/Thread";
import { Button, Skeleton } from "../ui";
import { HoverCard } from "../LongTailPrimitives";

export const ThinkingButton: React.FC = () => {
  const chatId = useThreadId();
  const isBoostReasoningEnabled = useAppSelector((state) =>
    selectThreadBoostReasoningById(state, chatId),
  );
  const {
    handleReasoningChange,
    shouldBeDisabled,
    noteText,
    areCapsInitialized,
    supportsBoostReasoning,
  } = useThinking();
  if (!areCapsInitialized) {
    return (
      <Skeleton height="var(--rf-control-h-sm)" radius="control" width="76px" />
    );
  }

  if (!supportsBoostReasoning) {
    return null;
  }

  return (
    <Flex gap="2" align="center">
      <HoverCard>
        <HoverCard.Trigger asChild>
          <Button
            leftIcon={WandSparkles}
            size="sm"
            onClick={(event) =>
              handleReasoningChange(event, !isBoostReasoningEnabled)
            }
            variant={isBoostReasoningEnabled ? "primary" : "soft"}
            disabled={shouldBeDisabled}
          >
            Think
          </Button>
        </HoverCard.Trigger>
        <HoverCard.Content maxWidth="500px" side="top">
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
      </HoverCard>
    </Flex>
  );
};
