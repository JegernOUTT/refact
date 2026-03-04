import React, { useState } from "react";
import { Badge, Box, Button, Flex, Text } from "@radix-ui/themes";
import {
  CheckIcon,
  ExternalLinkIcon,
  GearIcon,
  StarFilledIcon,
} from "@radix-ui/react-icons";
import type { MCPServer } from "../../services/refact/mcpMarketplace";
import styles from "./MCPMarketplace.module.css";

type ServerCardProps = {
  server: MCPServer;
  isInstalled: boolean;
  installedConfigPath?: string;
  onInstall: (server: MCPServer) => void;
  onViewDetail: (server: MCPServer) => void;
  onConfigure?: (configPath: string) => void;
  isInstalling: boolean;
  sourceLabel?: string;
};

export const ServerCard: React.FC<ServerCardProps> = ({
  server,
  isInstalled,
  installedConfigPath,
  onInstall,
  onViewDetail,
  onConfigure,
  isInstalling,
  sourceLabel,
}) => {
  const [imgError, setImgError] = useState(false);

  return (
    <Box className={styles.serverCard}>
      <Flex direction="column" gap="2" height="100%">
        <Flex align="center" gap="2">
          {server.icon_url && !imgError ? (
            <img
              src={server.icon_url}
              alt={server.name}
              className={styles.serverIcon}
              onError={() => setImgError(true)}
            />
          ) : (
            <Box className={styles.serverIconPlaceholder}>
              <Text size="4" weight="bold">
                {server.name.charAt(0).toUpperCase()}
              </Text>
            </Box>
          )}
          <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 0 }}>
            <Text size="2" weight="bold" truncate>
              {server.name}
            </Text>
            <Text size="1" color="gray" truncate>
              {server.publisher}
            </Text>
          </Flex>
          <Badge color="blue" variant="soft" size="1">
            {server.transport}
          </Badge>
        </Flex>

        <Text size="1" color="gray" className={styles.serverDescription}>
          {server.description}
        </Text>

        {server.tags.length > 0 && (
          <Flex gap="1" wrap="wrap">
            {server.tags.slice(0, 4).map((tag) => (
              <Badge key={tag} variant="soft" color="gray" size="1">
                {tag}
              </Badge>
            ))}
          </Flex>
        )}

        <Flex gap="2" mt="auto" align="center" wrap="wrap">
          {server.verified && (
            <Flex align="center" gap="1" className={styles.verifiedBadge}>
              <StarFilledIcon width={10} height={10} />
              <Text size="1">Verified</Text>
            </Flex>
          )}
          {server.use_count !== undefined && server.use_count > 0 && (
            <Text size="1" color="gray">
              {server.use_count} installs
            </Text>
          )}
          {sourceLabel && (
            <Badge
              color="gray"
              variant="soft"
              size="1"
              className={styles.sourceBadgeInCard}
            >
              {sourceLabel}
            </Badge>
          )}
        </Flex>
        <Flex gap="2" mt="2" align="center">
          {isInstalled ? (
            <>
              <Flex align="center" gap="1" style={{ flex: 1 }}>
                <CheckIcon color="var(--green-9)" />
                <Text size="1" color="green">
                  Installed
                </Text>
              </Flex>
              {installedConfigPath && onConfigure && (
                <Button
                  size="1"
                  variant="ghost"
                  onClick={() => onConfigure(installedConfigPath)}
                >
                  <GearIcon />
                  <Text size="1">Configure</Text>
                </Button>
              )}
            </>
          ) : (
            <Button
              size="1"
              onClick={() => onInstall(server)}
              disabled={isInstalling}
              style={{ flex: 1 }}
            >
              {isInstalling ? "Installing…" : "Install"}
            </Button>
          )}
          <Button size="1" variant="ghost" onClick={() => onViewDetail(server)}>
            {server.homepage ? (
              <ExternalLinkIcon />
            ) : (
              <Text size="1">Details</Text>
            )}
          </Button>
        </Flex>
      </Flex>
    </Box>
  );
};
