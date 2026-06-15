import React, { useState } from "react";
import classNames from "classnames";
import { Check, ExternalLink, Settings, Star } from "lucide-react";
import { Badge, Button, Card as KitCard, Icon } from "../../components/ui";
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
    <KitCard
      interactive
      className={classNames(
        styles.serverCard,
        "rf-glass-panel",
        isInstalling && styles.serverCardInstalling,
      )}
    >
      <div className={styles.cardColumn}>
        <div className={styles.cardBody}>
          <div className={styles.cardMeta}>
            {server.icon_url && !imgError ? (
              <img
                src={server.icon_url}
                alt={server.name}
                className={styles.serverIcon}
                onError={() => setImgError(true)}
              />
            ) : (
              <div className={styles.serverIconPlaceholder}>
                {server.name.charAt(0).toUpperCase()}
              </div>
            )}
            <div className={styles.cardTitle}>
              <p className={classNames(styles.text, styles.truncate)}>
                {server.name}
              </p>
              <p className={classNames(styles.smallText, styles.truncate)}>
                {server.publisher}
              </p>
            </div>
            <Badge tone="accent" className={styles.neutralBadge}>
              {server.transport}
            </Badge>
          </div>

          <p className={styles.serverDescription}>{server.description}</p>

          {server.tags.length > 0 && (
            <div className={styles.filterRow}>
              {server.tags.slice(0, 4).map((tag) => (
                <Badge key={tag} tone="muted">
                  {tag}
                </Badge>
              ))}
            </div>
          )}
        </div>

        <div className={styles.cardFooterGroup}>
          <div className={styles.cardFooter}>
            {server.verified && (
              <span
                className={classNames(styles.statusRow, styles.verifiedBadge)}
              >
                <Icon icon={Star} size="sm" tone="success" />
                <span className={styles.smallText}>Verified</span>
              </span>
            )}
            {server.use_count !== undefined && server.use_count > 0 && (
              <span className={styles.smallText}>
                {server.use_count} installs
              </span>
            )}
            {sourceLabel && (
              <Badge tone="muted" className={styles.sourceBadgeInCard}>
                {sourceLabel}
              </Badge>
            )}
          </div>
          <div className={styles.cardActionRow}>
            {isInstalled ? (
              <>
                <span
                  className={classNames(
                    styles.statusRow,
                    styles.grow,
                    styles.successText,
                  )}
                >
                  <Icon icon={Check} size="sm" tone="success" />
                  <span className={styles.smallText}>Installed</span>
                </span>
                {installedConfigPath && onConfigure && (
                  <Button
                    size="sm"
                    variant="ghost"
                    leftIcon={Settings}
                    onClick={() => onConfigure(installedConfigPath)}
                  >
                    Configure
                  </Button>
                )}
              </>
            ) : (
              <Button
                size="sm"
                variant="primary"
                onClick={() => onInstall(server)}
                disabled={isInstalling}
                loading={isInstalling}
                className={styles.grow}
              >
                {isInstalling ? "Installing…" : "Install"}
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              rightIcon={server.homepage ? ExternalLink : undefined}
              onClick={() => onViewDetail(server)}
            >
              Details
            </Button>
          </div>
        </div>
      </div>
    </KitCard>
  );
};
