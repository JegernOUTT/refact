import React, { useState } from "react";
import { Badge, Box, Flex, Switch, Text } from "@radix-ui/themes";
import {
  CounterClockwiseClockIcon,
  ExclamationTriangleIcon,
  LockClosedIcon,
} from "@radix-ui/react-icons";
import type { MCPToolInfo } from "../../../services/refact/mcpServerInfo";
import styles from "./MCPToolsList.module.css";

type MCPToolsListProps = {
  tools: MCPToolInfo[];
};

const AnnotationBadges: React.FC<{
  annotations?: MCPToolInfo["annotations"];
}> = ({ annotations }) => {
  if (!annotations) return null;
  return (
    <Flex gap="1" wrap="wrap">
      {annotations.readOnlyHint && (
        <Badge size="1">
          <LockClosedIcon /> readOnly
        </Badge>
      )}
      {annotations.destructiveHint && (
        <Badge size="1">
          <ExclamationTriangleIcon /> destructive
        </Badge>
      )}
      {annotations.idempotentHint && (
        <Badge size="1">
          <CounterClockwiseClockIcon /> idempotent
        </Badge>
      )}
    </Flex>
  );
};

const MCPToolRow: React.FC<{ tool: MCPToolInfo }> = ({ tool }) => {
  const [enabled, setEnabled] = useState(true);
  const [expanded, setExpanded] = useState(false);

  return (
    <Box className={styles.toolRow}>
      <Flex align="start" gap="3">
        <Switch
          size="1"
          checked={enabled}
          onCheckedChange={setEnabled}
          aria-label={`Toggle ${tool.name}`}
        />
        <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 0 }}>
          <Flex align="center" gap="2" wrap="wrap">
            <Text size="2" weight="medium">
              {tool.name}
            </Text>
            <AnnotationBadges annotations={tool.annotations} />
          </Flex>
          {tool.description && (
            <Text size="1" color="gray">
              {tool.description}
            </Text>
          )}
          <button
            className={styles.expandButton}
            onClick={() => setExpanded(!expanded)}
            type="button"
          >
            <Text size="1" color="gray">
              {expanded ? "Hide schema" : "Show schema"}
            </Text>
          </button>
          {expanded && (
            <Box className={styles.schemaBox}>
              <pre className={styles.schemaPre}>
                {JSON.stringify(tool.input_schema, null, 2)}
              </pre>
            </Box>
          )}
        </Flex>
      </Flex>
    </Box>
  );
};

export const MCPToolsList: React.FC<MCPToolsListProps> = ({ tools }) => {
  if (tools.length === 0) {
    return (
      <Text size="2" color="gray">
        No tools available
      </Text>
    );
  }

  return (
    <Flex direction="column" gap="1">
      {tools.map((tool) => (
        <MCPToolRow key={tool.name} tool={tool} />
      ))}
    </Flex>
  );
};
