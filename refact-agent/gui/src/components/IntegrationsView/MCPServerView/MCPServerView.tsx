import React, { useState } from "react";
import {
  Bolt,
  ChevronDown,
  ChevronRight,
  FileText,
  ListTree,
  ScrollText,
  Settings,
} from "lucide-react";
import {
  useGetMCPServerInfoQuery,
  useReconnectMCPServerMutation,
} from "../../../services/refact/mcpServerInfo";
import { MCPConnectionStatus } from "./MCPConnectionStatus";
import { MCPToolsList } from "./MCPToolsList";
import { MCPResourcesList } from "./MCPResourcesList";
import { MCPPromptsList } from "./MCPPromptsList";
import { MCPLogs } from "../IntegrationForm/MCPLogs";
import { MCPOAuth } from "./MCPOAuth";
import { toPascalCase } from "../../../utils/toPascalCase";
import { Button, Flex, Icon, Spinner, Surface, Text } from "../../ui";
import styles from "./MCPServerView.module.css";

type CollapsibleSectionProps = {
  icon?: React.ReactNode;
  title: string;
  count?: number;
  defaultExpanded?: boolean;
  children: React.ReactNode;
};

const CollapsibleSection: React.FC<CollapsibleSectionProps> = ({
  icon,
  title,
  count,
  defaultExpanded = false,
  children,
}) => {
  const [expanded, setExpanded] = useState(defaultExpanded);

  return (
    <Surface
      animated="rise"
      className={styles.section}
      radius="card"
      variant="glass"
    >
      <button
        className={styles.sectionHeader}
        onClick={() => setExpanded(!expanded)}
        type="button"
        aria-expanded={expanded}
      >
        <Flex align="center" gap="2">
          {icon}
          <Text size="2" weight="medium">
            {title}
          </Text>
          {count !== undefined && (
            <Text size="1" color="gray">
              ({count})
            </Text>
          )}
        </Flex>
        <Icon icon={expanded ? ChevronDown : ChevronRight} size="sm" />
      </button>
      <div
        className="rf-expand-grid"
        data-open={expanded ? true : undefined}
        data-state={expanded ? "open" : "closed"}
      >
        <div>
          <div className={styles.sectionContent}>{children}</div>
        </div>
      </div>
    </Surface>
  );
};

type MCPServerViewProps = {
  configPath: string;
  integrName: string;
};

export const MCPServerView: React.FC<MCPServerViewProps> = ({
  configPath,
  integrName,
}) => {
  const { data, isLoading, isError } = useGetMCPServerInfoQuery(
    { configPath },
    { pollingInterval: 3000 },
  );
  const [reconnect, { isLoading: isReconnecting }] =
    useReconnectMCPServerMutation();

  const handleReconnect = () => {
    void reconnect({ configPath });
  };

  if (isLoading) {
    return (
      <Surface
        animated="rise"
        className={styles.loadingSurface}
        radius="card"
        variant="glass"
      >
        <Flex align="center" justify="center" gap="2">
          <Spinner size="sm" />
          <Text size="2" color="gray">
            Loading MCP server info...
          </Text>
        </Flex>
      </Surface>
    );
  }

  if (isError || !data) {
    return (
      <Surface
        animated="rise"
        className={styles.unavailableSurface}
        radius="card"
        variant="glass"
      >
        <Flex direction="column" gap="3">
          <Text as="p" size="2" color="gray">
            MCP server info not available. The server may not be connected yet.
          </Text>
          <Flex align="center" gap="2" wrap="wrap">
            <Button
              size="md"
              variant="soft"
              onClick={handleReconnect}
              disabled={isReconnecting}
            >
              {isReconnecting ? "Reconnecting..." : "Reconnect"}
            </Button>
            {isReconnecting && <Spinner size="sm" />}
          </Flex>
          <div className={styles.divider} role="separator" />
          <MCPLogs
            integrationPath={configPath}
            integrationName={toPascalCase(integrName)}
          />
        </Flex>
      </Surface>
    );
  }

  return (
    <Flex className={`${styles.root} rf-enter`} direction="column" gap="3">
      <Flex align="center" justify="between" wrap="wrap" gap="2">
        <h3 className={styles.title}>
          {data.server_name ?? toPascalCase(integrName)}
          {data.server_version && (
            <Text size="2" color="gray" className={styles.version}>
              v{data.server_version}
            </Text>
          )}
        </h3>
      </Flex>

      {data.protocol_version && (
        <Text size="1" color="gray">
          Protocol: {data.protocol_version}
        </Text>
      )}

      <div className={styles.divider} role="separator" />

      <MCPOAuth configPath={configPath} />

      <div className="rf-stagger">
        <CollapsibleSection
          icon={<Icon icon={Bolt} size="sm" />}
          title="Connection"
          defaultExpanded
        >
          <MCPConnectionStatus
            status={data.status}
            onReconnect={handleReconnect}
            isReconnecting={isReconnecting}
          />
        </CollapsibleSection>

        <CollapsibleSection
          icon={<Icon icon={Settings} size="sm" />}
          title="Tools"
          count={data.tools.length}
          defaultExpanded
        >
          <MCPToolsList tools={data.tools} />
        </CollapsibleSection>

        {data.resources.length > 0 && (
          <CollapsibleSection
            icon={<Icon icon={ListTree} size="sm" />}
            title="Resources"
            count={data.resources.length}
          >
            <MCPResourcesList resources={data.resources} />
          </CollapsibleSection>
        )}

        {data.prompts.length > 0 && (
          <CollapsibleSection
            icon={<Icon icon={FileText} size="sm" />}
            title="Prompts"
            count={data.prompts.length}
          >
            <MCPPromptsList prompts={data.prompts} />
          </CollapsibleSection>
        )}

        <CollapsibleSection icon={<Icon icon={ScrollText} size="sm" />} title="Logs">
          <MCPLogs
            integrationPath={configPath}
            integrationName={toPascalCase(integrName)}
          />
        </CollapsibleSection>
      </div>
    </Flex>
  );
};
