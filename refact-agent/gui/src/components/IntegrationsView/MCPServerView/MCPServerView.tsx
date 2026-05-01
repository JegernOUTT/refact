import React, { useState } from "react";
import {
  Box,
  Button,
  Flex,
  Heading,
  Separator,
  Spinner,
  Text,
} from "@radix-ui/themes";
import {
  FileTextIcon,
  GearIcon,
  LightningBoltIcon,
  ReaderIcon,
  RowsIcon,
} from "@radix-ui/react-icons";
import { useAppSelector } from "../../../hooks";
import { selectLspPort } from "../../../features/Config/configSlice";
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
    <Box>
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
        <Text size="1" color="gray">
          {expanded ? "▲" : "▼"}
        </Text>
      </button>
      {expanded && <Box className={styles.sectionContent}>{children}</Box>}
    </Box>
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
  const port = useAppSelector(selectLspPort);
  const { data, isLoading, isError } = useGetMCPServerInfoQuery(
    { configPath, port },
    { pollingInterval: 3000 },
  );
  const [reconnect, { isLoading: isReconnecting }] =
    useReconnectMCPServerMutation();

  const handleReconnect = () => {
    void reconnect({ configPath, port });
  };

  if (isLoading) {
    return (
      <Flex p="4" align="center" justify="center" gap="2">
        <Spinner size="2" />
        <Text size="2" color="gray">
          Loading MCP server info...
        </Text>
      </Flex>
    );
  }

  if (isError || !data) {
    return (
      <Flex direction="column" gap="3" p="4">
        <Text size="2" color="gray">
          MCP server info not available. The server may not be connected yet.
        </Text>
        <Flex align="center" gap="2">
          <Button
            size="2"
            variant="soft"
            onClick={handleReconnect}
            disabled={isReconnecting}
          >
            {isReconnecting ? "Reconnecting..." : "Reconnect"}
          </Button>
          {isReconnecting && <Spinner size="2" />}
        </Flex>
        <Separator size="4" />
        <MCPLogs
          integrationPath={configPath}
          integrationName={toPascalCase(integrName)}
        />
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="3" pb="8">
      <Flex align="center" justify="between" wrap="wrap" gap="2">
        <Heading size="3">
          {data.server_name ?? toPascalCase(integrName)}
          {data.server_version && (
            <Text size="2" color="gray" ml="2">
              v{data.server_version}
            </Text>
          )}
        </Heading>
      </Flex>

      {data.protocol_version && (
        <Text size="1" color="gray">
          Protocol: {data.protocol_version}
        </Text>
      )}

      <Separator size="4" />

      <MCPOAuth configPath={configPath} />

      <CollapsibleSection
        icon={<LightningBoltIcon />}
        title="Connection"
        defaultExpanded
      >
        <MCPConnectionStatus
          status={data.status}
          onReconnect={handleReconnect}
          isReconnecting={isReconnecting}
        />
      </CollapsibleSection>

      <Separator size="4" />

      <CollapsibleSection
        icon={<GearIcon />}
        title="Tools"
        count={data.tools.length}
        defaultExpanded
      >
        <MCPToolsList tools={data.tools} />
      </CollapsibleSection>

      {data.resources.length > 0 && (
        <>
          <Separator size="4" />
          <CollapsibleSection
            icon={<RowsIcon />}
            title="Resources"
            count={data.resources.length}
          >
            <MCPResourcesList resources={data.resources} />
          </CollapsibleSection>
        </>
      )}

      {data.prompts.length > 0 && (
        <>
          <Separator size="4" />
          <CollapsibleSection
            icon={<FileTextIcon />}
            title="Prompts"
            count={data.prompts.length}
          >
            <MCPPromptsList prompts={data.prompts} />
          </CollapsibleSection>
        </>
      )}

      <Separator size="4" />

      <CollapsibleSection icon={<ReaderIcon />} title="Logs">
        <MCPLogs
          integrationPath={configPath}
          integrationName={toPascalCase(integrName)}
        />
      </CollapsibleSection>
    </Flex>
  );
};
