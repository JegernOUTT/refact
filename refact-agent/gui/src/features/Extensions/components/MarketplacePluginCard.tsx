import React, { useCallback } from "react";
import { Check } from "lucide-react";
import { Badge, Button, FieldError, Icon } from "../../../components/ui";
import {
  useInstallPluginMutation,
  useUninstallPluginMutation,
} from "../../../services/refact/plugins";
import type { PluginEntry } from "../../../services/refact/plugins";

import styles from "./MarketplacePluginCard.module.css";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringifyMutationValue(value: unknown, fallback: string): string {
  if (typeof value === "string") {
    return value === "[object Object]" ? fallback : value;
  }
  if (value == null) return fallback;
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value);
  }
  try {
    return JSON.stringify(value);
  } catch {
    return fallback;
  }
}

function getMutationErrorMessage(error: unknown, fallback: string): string {
  if (!isRecord(error)) return fallback;
  if ("data" in error) return stringifyMutationValue(error.data, fallback);
  if ("message" in error)
    return stringifyMutationValue(error.message, fallback);
  return fallback;
}

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
      ? getMutationErrorMessage(installError, "Install failed")
      : uninstallError != null
        ? getMutationErrorMessage(uninstallError, "Uninstall failed")
        : null;

  return (
    <article className={`${styles.card} rf-glass-panel rf-pressable`}>
      <div className={styles.body}>
        <div className={styles.header}>
          <div className={styles.info}>
            <h3 className={styles.title}>{plugin.name}</h3>
            {plugin.description && (
              <p className={styles.description}>{plugin.description}</p>
            )}
          </div>
          <div className={styles.actions}>
            {isInstalled ? (
              <div className={styles.installed}>
                <Icon icon={Check} size="sm" tone="success" />
                Installed
                <Button
                  size="sm"
                  variant="soft"
                  onClick={handleUninstall}
                  disabled={uninstalling}
                  loading={uninstalling}
                >
                  Uninstall
                </Button>
              </div>
            ) : (
              <Button
                size="sm"
                variant="primary"
                onClick={handleInstall}
                disabled={installing}
                loading={installing}
              >
                Install
              </Button>
            )}
          </div>
        </div>

        {errorMessage && <FieldError>{errorMessage}</FieldError>}

        <div className={styles.tags}>
          <Badge tone="muted">{plugin.marketplace}</Badge>
          {plugin.version && <Badge tone="muted">{plugin.version}</Badge>}
          {plugin.tags?.map((tag) => (
            <Badge key={tag} tone="default">
              {tag}
            </Badge>
          ))}
        </div>
      </div>
    </article>
  );
};
