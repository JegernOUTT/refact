import React, { useEffect, useMemo, useState } from "react";
import classNames from "classnames";
import { Lock, RotateCcw, TriangleAlert } from "lucide-react";
import type { MCPToolInfo } from "../../../services/refact/mcpServerInfo";
import { integrationsApi } from "../../../services/refact/integrations";
import { Badge, Flex, Icon, Surface, Switch, Text } from "../../ui";
import styles from "./MCPToolsList.module.css";

type MCPToolsListProps = {
  tools: MCPToolInfo[];
  configPath?: string;
};

const normalizeToolSet = (value: unknown): Set<string> => {
  if (Array.isArray(value)) {
    return new Set(
      value.filter((item): item is string => typeof item === "string"),
    );
  }

  if (typeof value === "string") {
    return new Set(
      value
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean),
    );
  }

  return new Set();
};

const serializeToolSet = (value: Set<string>) => Array.from(value).join(",");

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

const MCPToolRow: React.FC<{
  tool: MCPToolInfo;
  enabled: boolean;
  autoApproved: boolean;
  showControls: boolean;
  onEnabledChange: (checked: boolean) => void;
  onAutoApproveChange: (checked: boolean) => void;
}> = ({
  tool,
  enabled,
  autoApproved,
  showControls,
  onEnabledChange,
  onAutoApproveChange,
}) => {
  const [expanded, setExpanded] = useState(false);

  return (
    <Surface
      animated="rise"
      className={classNames(styles.toolRow, !enabled && styles.toolRowDisabled)}
      radius="control"
      variant="plain"
    >
      <Flex align="start" gap="3" justify="between">
        <Flex className={styles.toolBody} direction="column" gap="1">
          <Flex
            align="center"
            className={!enabled ? styles.toolContentDisabled : undefined}
            gap="2"
            wrap="wrap"
          >
            <Text size="2" weight="medium">
              {tool.name}
            </Text>
            <AnnotationBadges annotations={tool.annotations} />
          </Flex>
          {tool.description && (
            <Text
              as="p"
              className={!enabled ? styles.toolContentDisabled : undefined}
              size="1"
              color="gray"
            >
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
        {showControls && (
          <Flex className={styles.controls} align="center" gap="3" wrap="wrap">
            <Switch
              aria-label={`${tool.name} Enabled`}
              checked={enabled}
              label="Enabled"
              onCheckedChange={onEnabledChange}
            />
            <Switch
              aria-label={`${tool.name} Auto-approve`}
              checked={enabled && autoApproved}
              disabled={!enabled}
              label="Auto-approve"
              onCheckedChange={onAutoApproveChange}
              title={!enabled ? "Enable the tool first" : undefined}
            />
          </Flex>
        )}
      </Flex>
    </Surface>
  );
};

export const MCPToolsList: React.FC<MCPToolsListProps> = ({
  tools,
  configPath,
}) => {
  const { data: integrationData } =
    integrationsApi.useGetIntegrationByPathQuery(configPath ?? "", {
      skip: !configPath,
    });
  const [saveIntegration] = integrationsApi.useSaveIntegrationMutation();
  const [disabledTools, setDisabledTools] = useState<Set<string>>(new Set());
  const [autoApproveTools, setAutoApproveTools] = useState<Set<string>>(
    new Set(),
  );
  const [error, setError] = useState<string | null>(null);

  const values = useMemo(
    () => integrationData?.integr_values ?? null,
    [integrationData?.integr_values],
  );

  useEffect(() => {
    if (!values) return;

    const rawValues = values as Record<string, unknown>;
    setDisabledTools(normalizeToolSet(rawValues.disabled_tools));
    setAutoApproveTools(normalizeToolSet(rawValues.auto_approve_tools));
  }, [values]);

  const saveToolSet = async (
    field: "disabled_tools" | "auto_approve_tools",
    nextSet: Set<string>,
    rollback: () => void,
  ) => {
    if (!values || !configPath) {
      rollback();
      setError(
        "Unable to save tool setting: integration config is not loaded.",
      );
      return;
    }

    setError(null);
    const nextDisabled = field === "disabled_tools" ? nextSet : disabledTools;
    const nextAutoApprove =
      field === "auto_approve_tools" ? nextSet : autoApproveTools;
    const result = await saveIntegration({
      filePath: configPath,
      values: {
        ...values,
        disabled_tools: serializeToolSet(nextDisabled),
        auto_approve_tools: serializeToolSet(nextAutoApprove),
      },
    });

    if (result.error) {
      rollback();
      setError("Unable to save tool setting. Please try again.");
    }
  };

  const handleEnabledChange = (toolName: string, checked: boolean) => {
    const previousDisabled = new Set(disabledTools);
    const nextDisabled = new Set(disabledTools);
    if (checked) {
      nextDisabled.delete(toolName);
    } else {
      nextDisabled.add(toolName);
    }
    setDisabledTools(nextDisabled);
    void saveToolSet("disabled_tools", nextDisabled, () => {
      setDisabledTools(previousDisabled);
    });
  };

  const handleAutoApproveChange = (toolName: string, checked: boolean) => {
    const previousAutoApprove = new Set(autoApproveTools);
    const nextAutoApprove = new Set(autoApproveTools);
    if (checked) {
      nextAutoApprove.add(toolName);
    } else {
      nextAutoApprove.delete(toolName);
    }
    setAutoApproveTools(nextAutoApprove);
    void saveToolSet("auto_approve_tools", nextAutoApprove, () => {
      setAutoApproveTools(previousAutoApprove);
    });
  };

  if (tools.length === 0) {
    return (
      <Text size="2" color="gray">
        No tools available
      </Text>
    );
  }

  return (
    <Flex className="rf-stagger" direction="column" gap="1">
      <Text size="1" color="gray">
        Disabled tools are hidden from the model. Auto-approved tools run
        without confirmation.
      </Text>
      {error && (
        <Text size="1" color="red">
          {error}
        </Text>
      )}
      {tools.map((tool) => (
        <MCPToolRow
          key={tool.name}
          tool={tool}
          enabled={!disabledTools.has(tool.name)}
          autoApproved={autoApproveTools.has(tool.name)}
          showControls={Boolean(configPath)}
          onEnabledChange={(checked) => handleEnabledChange(tool.name, checked)}
          onAutoApproveChange={(checked) =>
            handleAutoApproveChange(tool.name, checked)
          }
        />
      ))}
    </Flex>
  );
};
