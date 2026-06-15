import React from "react";
import type { MCPResourceInfo } from "../../../services/refact/mcpServerInfo";
import { Flex, Text } from "../../ui";
import styles from "./MCPServerView.module.css";

type MCPResourcesListProps = {
  resources: MCPResourceInfo[];
};

export const MCPResourcesList: React.FC<MCPResourcesListProps> = ({
  resources,
}) => {
  if (resources.length === 0) {
    return (
      <Text size="2" color="gray">
        No resources available
      </Text>
    );
  }

  return (
    <Flex className={styles.list} direction="column" gap="2">
      {resources.map((resource) => (
        <Flex
          key={resource.uri}
          className="rf-enter-rise"
          direction="column"
          gap="1"
        >
          <Flex className={styles.listItem} gap="2" align="center" wrap="wrap">
            <Text className={styles.resourceName} size="2" weight="medium">
              {resource.uri}
            </Text>
            {resource.mime_type && (
              <Text size="1" color="gray">
                {resource.mime_type}
              </Text>
            )}
          </Flex>
          {resource.description && (
            <Text as="p" size="1" color="gray">
              {resource.description}
            </Text>
          )}
        </Flex>
      ))}
    </Flex>
  );
};
