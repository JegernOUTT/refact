import React from "react";
import type { MCPPromptInfo } from "../../../services/refact/mcpServerInfo";
import { Flex, Text } from "../../ui";
import styles from "./MCPServerView.module.css";

type MCPPromptsListProps = {
  prompts: MCPPromptInfo[];
};

export const MCPPromptsList: React.FC<MCPPromptsListProps> = ({ prompts }) => {
  if (prompts.length === 0) {
    return (
      <Text size="2" color="gray">
        No prompts available
      </Text>
    );
  }

  return (
    <Flex className={styles.list} direction="column" gap="2">
      {prompts.map((prompt) => (
        <Flex
          key={prompt.name}
          className="rf-enter-rise"
          direction="column"
          gap="1"
        >
          <Text size="2" weight="medium">
            {prompt.name}
          </Text>
          {prompt.description && (
            <Text as="p" size="1" color="gray">
              {prompt.description}
            </Text>
          )}
        </Flex>
      ))}
    </Flex>
  );
};
