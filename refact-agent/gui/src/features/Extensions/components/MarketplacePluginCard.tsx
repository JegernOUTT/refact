import React, { useCallback } from "react";
import { Badge, Button, Card, Flex, Spinner, Text } from "@radix-ui/themes";
import {
  useInstallPluginMutation,
  useUninstallPluginMutation,
} from "../../../services/refact/plugins";
import type { PluginEntry } from "../../../services/refact/plugins";

import styles from "./MarketplacePluginCard.module.css";

export type MarketplacePluginCardProps = {
  plugin: PluginEntry;
  isInstalled: boolean;
};

export const MarketplacePluginCard: React.FC<MarketplacePluginCardProps> = ({
  plugin,
  isInstalled,
}) => {
  const [installPlugin, { isLoading: installing, error: installError }] =
    useInstallPluginMutation();
  const [uninstallPlugin, { isLoading: uninstalling, error: uninstallError }] =
    useUninstallPluginMutation();

  const handleInstall = useCallback(() => {
    void installPlugin({
      plugin: plugin.name,
      marketplace: plugin.marketplace,
    });
  }, [installPlugin, plugin.name, plugin.marketplace]);

  const handleUninstall = useCallback(() => {
    void uninstallPlugin(plugin.name);
  }, [uninstallPlugin, plugin.name]);

  const errorMessage =
    installError != null
      ? String(
          "data" in installError
            ? installError.data
            : "message" in installError
              ? installError.message
              : "Install failed",
        )
      : uninstallError != null
        ? String(
            "data" in uninstallError
              ? uninstallError.data
              : "message" in uninstallError
                ? uninstallError.message
                : "Uninstall failed",
          )
        : null;

  return (
    <Card className={styles.card}>
      <Flex direction="column" gap="2" height="100%">
        <div className={styles.header}>
          <div className={styles.info}>
            <Text size="2" weight="bold">
              {plugin.name}
            </Text>
            {plugin.description && (
              <Text size="1" className={styles.description} mt="1" as="p">
                {plugin.description}
              </Text>
            )}
          </div>
          <div className={styles.actions}>
            {isInstalled ? (
              <Flex gap="2" align="center">
                <Text size="1" weight="medium">
                  Installed ✓
                </Text>
                <Button
                  size="1"
                  variant="soft"
                  onClick={handleUninstall}
                  disabled={uninstalling}
                >
                  {uninstalling ? <Spinner size="1" /> : "Uninstall"}
                </Button>
              </Flex>
            ) : (
              <Button size="1" onClick={handleInstall} disabled={installing}>
                {installing ? <Spinner size="1" /> : "Install"}
              </Button>
            )}
          </div>
        </div>

        {errorMessage && (
          <Text size="1" color="red">
            {errorMessage}
          </Text>
        )}

        <Flex className={styles.tags} gap="1" wrap="wrap">
          <Badge size="1" color="gray" variant="soft">
            {plugin.marketplace}
          </Badge>
          {plugin.version && (
            <Badge size="1" variant="soft">
              {plugin.version}
            </Badge>
          )}
          {plugin.tags?.map((tag) => (
            <Badge key={tag} size="1" variant="soft">
              {tag}
            </Badge>
          ))}
        </Flex>
      </Flex>
    </Card>
  );
};
