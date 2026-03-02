import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import type { MCPPromptInfo } from "../../../services/refact/mcpServerInfo";

type MCPPromptsListProps = {
  prompts: MCPPromptInfo[];
};

export const MCPPromptsList: React.FC<MCPPromptsListProps> = ({ prompts }) => {
  if (prompts.length === 0) {
    return <Text size="2" color="gray">No prompts available</Text>;
  }

  return (
    <Flex direction="column" gap="2">
      {prompts.map((prompt) => (
        <Flex key={prompt.name} direction="column" gap="1">
          <Text size="2" weight="medium">{prompt.name}</Text>
          {prompt.description && (
            <Text size="1" color="gray">{prompt.description}</Text>
          )}
        </Flex>
      ))}
    </Flex>
  );
};
