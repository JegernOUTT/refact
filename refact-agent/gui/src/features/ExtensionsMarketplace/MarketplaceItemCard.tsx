import React from "react";
import { Badge, Box, Button, Flex, Text } from "@radix-ui/themes";
import { CheckIcon, ExternalLinkIcon } from "@radix-ui/react-icons";
import type { ExtensionMarketplaceItem } from "../../services/refact/extensionsMarketplace";
import styles from "./ExtensionsMarketplace.module.css";

type MarketplaceItemCardProps = {
  item: ExtensionMarketplaceItem;
  isInstalling: boolean;
  onInstall: (item: ExtensionMarketplaceItem) => void;
};

export const MarketplaceItemCard: React.FC<MarketplaceItemCardProps> = ({
  item,
  isInstalling,
  onInstall,
}) => {
  return (
    <Box className={styles.card}>
      <Flex direction="column" gap="2" height="100%">
        <Flex align="center" gap="2">
          <Flex direction="column" gap="1" style={{ flex: 1, minWidth: 0 }}>
            <Text size="2" weight="bold" truncate>
              {item.name}
            </Text>
            <Text size="1" color="gray" truncate>
              {item.publisher}
            </Text>
          </Flex>
          <Badge color="blue" variant="soft" size="1">
            {item.kind}
          </Badge>
        </Flex>

        <Text size="1" color="gray" className={styles.description}>
          {item.description || "No description"}
        </Text>

        {item.body_preview && (
          <Text size="1" color="gray" className={styles.bodyPreview}>
            {item.body_preview}
          </Text>
        )}

        {item.tags.length > 0 && (
          <Flex gap="1" wrap="wrap">
            {item.tags.slice(0, 4).map((tag) => (
              <Badge key={tag} variant="soft" color="gray" size="1">
                {tag}
              </Badge>
            ))}
          </Flex>
        )}

        <Flex gap="2" mt="auto" align="center" wrap="wrap">
          <Badge
            color="gray"
            variant="soft"
            size="1"
            className={styles.sourceBadge}
          >
            {item.source_label}
          </Badge>
          {item.installed_scopes.length > 0 && (
            <Flex align="center" gap="1">
              <CheckIcon color="var(--green-9)" />
              <Text size="1" color="green">
                Installed: {item.installed_scopes.join(", ")}
              </Text>
            </Flex>
          )}
        </Flex>

        <Flex gap="2" mt="2" align="center">
          <Button
            size="1"
            onClick={() => onInstall(item)}
            disabled={isInstalling}
            style={{ flex: 1 }}
          >
            {isInstalling ? "Installing…" : "Install"}
          </Button>
          {item.homepage && (
            <Button
              size="1"
              variant="ghost"
              onClick={() =>
                window.open(item.homepage, "_blank", "noopener,noreferrer")
              }
            >
              <ExternalLinkIcon />
            </Button>
          )}
        </Flex>
      </Flex>
    </Box>
  );
};
