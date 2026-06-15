import React, { useState } from "react";
import classNames from "classnames";
import { Lock, RotateCcw, TriangleAlert } from "lucide-react";
import type { MCPToolInfo } from "../../../services/refact/mcpServerInfo";
import { Badge, Flex, Icon, Surface, Text } from "../../ui";
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
        <Badge tone="muted">
          <Icon icon={Lock} size="sm" /> readOnly
        </Badge>
      )}
      {annotations.destructiveHint && (
        <Badge tone="danger">
          <Icon icon={TriangleAlert} size="sm" /> destructive
        </Badge>
      )}
      {annotations.idempotentHint && (
        <Badge tone="muted">
          <Icon icon={RotateCcw} size="sm" /> idempotent
        </Badge>
      )}
    </Flex>
  );
};

const MCPToolRow: React.FC<{ tool: MCPToolInfo }> = ({ tool }) => {
  const [expanded, setExpanded] = useState(false);

  return (
    <Surface
      animated="rise"
      className={styles.toolRow}
      radius="control"
      variant="plain"
    >
      <Flex align="start" gap="3">
        <Flex className={styles.toolBody} direction="column" gap="1">
          <Flex align="center" gap="2" wrap="wrap">
            <Text size="2" weight="medium">
              {tool.name}
            </Text>
            <AnnotationBadges annotations={tool.annotations} />
          </Flex>
          {tool.description && (
            <Text as="p" size="1" color="gray">
              {tool.description}
            </Text>
          )}
          <button
            className={classNames(styles.expandButton, "rf-pressable")}
            onClick={() => setExpanded(!expanded)}
            type="button"
          >
            <Text size="1" color="gray">
              {expanded ? "Hide schema" : "Show schema"}
            </Text>
          </button>
          <div
            className="rf-expand-grid"
            data-open={expanded ? true : undefined}
            data-state={expanded ? "open" : "closed"}
          >
            <div>
              <div className={styles.schemaBox}>
                <pre className={styles.schemaPre}>
                  {JSON.stringify(tool.input_schema, null, 2)}
                </pre>
              </div>
            </div>
          </div>
        </Flex>
      </Flex>
    </Surface>
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
    <Flex className="rf-stagger" direction="column" gap="1">
      {tools.map((tool) => (
        <MCPToolRow key={tool.name} tool={tool} />
      ))}
    </Flex>
  );
};
