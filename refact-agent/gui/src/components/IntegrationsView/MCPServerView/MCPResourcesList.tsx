import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import type { MCPResourceInfo } from "../../../services/refact/mcpServerInfo";

type MCPResourcesListProps = {
  resources: MCPResourceInfo[];
};

export const MCPResourcesList: React.FC<MCPResourcesListProps> = ({ resources }) => {
  if (resources.length === 0) {
    return <Text size="2" color="gray">No resources available</Text>;
  }

  return (
    <Flex direction="column" gap="2">
      {resources.map((resource) => (
        <Flex key={resource.uri} direction="column" gap="1">
          <Flex gap="2" align="center">
            <Text size="2" weight="medium" style={{ fontFamily: "var(--font-mono)" }}>
              {resource.uri}
            </Text>
            {resource.mime_type && (
              <Text size="1" color="gray">{resource.mime_type}</Text>
            )}
          </Flex>
          {resource.description && (
            <Text size="1" color="gray">{resource.description}</Text>
          )}
        </Flex>
      ))}
    </Flex>
  );
};
