import React, { useCallback } from "react";
import classNames from "classnames";
import { Check } from "lucide-react";
import {
  Badge,
  Button,
  Card as KitCard,
  FieldError,
  Icon,
} from "../../../components/ui";
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
    <KitCard interactive className={classNames(styles.card, "rf-glass-panel")}>
      <div className={styles.cardColumn}>
        <div className={styles.cardBody}>
          <div className={styles.cardMeta}>
            <div className={styles.cardTitle}>
              <p className={classNames(styles.text, styles.truncate)}>
                {plugin.name}
              </p>
            </div>
            {plugin.version && (
              <Badge tone="accent" className={styles.neutralBadge}>
                {plugin.version}
              </Badge>
            )}
          </div>

          <p className={styles.description}>
            {plugin.description || "No description"}
          </p>

          {plugin.tags && plugin.tags.length > 0 && (
            <div className={styles.filterRow}>
              {plugin.tags.slice(0, 4).map((tag) => (
                <Badge key={tag} tone="muted">
                  {tag}
                </Badge>
              ))}
            </div>
          )}

          {errorMessage && <FieldError>{errorMessage}</FieldError>}
        </div>

        <div className={styles.cardFooterGroup}>
          <div className={styles.cardFooter}>
            <Badge tone="muted" className={styles.sourceBadge}>
              {plugin.marketplace}
            </Badge>
            {isInstalled && (
              <span
                className={classNames(styles.cardActionRow, styles.successText)}
              >
                <Icon icon={Check} size="sm" tone="success" />
                <span className={styles.smallText}>Installed</span>
              </span>
            )}
          </div>

          <div className={styles.cardActionRow}>
            {isInstalled ? (
              <Button
                size="sm"
                variant="soft"
                onClick={handleUninstall}
                disabled={uninstalling}
                loading={uninstalling}
                className={styles.grow}
              >
                Uninstall
              </Button>
            ) : (
              <Button
                size="sm"
                variant="primary"
                onClick={handleInstall}
                disabled={installing}
                loading={installing}
                className={styles.grow}
              >
                Install
              </Button>
            )}
          </div>
        </div>
      </div>
    </KitCard>
  );
};
