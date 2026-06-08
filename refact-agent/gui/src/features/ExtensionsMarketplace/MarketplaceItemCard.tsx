import React from "react";
import classNames from "classnames";
import { Check, ExternalLink } from "lucide-react";
import { Badge, Button, Card as KitCard, Icon } from "../../components/ui";
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
    <KitCard
      animated
      className={classNames(styles.card, isInstalling && styles.cardInstalling)}
    >
      <div className={styles.cardColumn}>
        <div className={styles.cardMeta}>
          <div className={styles.cardTitle}>
            <p className={classNames(styles.text, styles.truncate)}>
              {item.name}
            </p>
            <p className={classNames(styles.smallText, styles.truncate)}>
              {item.publisher}
            </p>
          </div>
          <Badge tone="accent">{item.kind}</Badge>
        </div>

        <p className={styles.description}>
          {item.description || "No description"}
        </p>

        {item.body_preview && (
          <p className={styles.bodyPreview}>{item.body_preview}</p>
        )}

        {item.tags.length > 0 && (
          <div className={styles.filterRow}>
            {item.tags.slice(0, 4).map((tag) => (
              <Badge key={tag} tone="muted">
                {tag}
              </Badge>
            ))}
          </div>
        )}

        <div className={styles.cardFooter}>
          <Badge tone="muted" className={styles.sourceBadge}>
            {item.source_label}
          </Badge>
          {item.installed_scopes.length > 0 && (
            <span
              className={classNames(styles.cardActionRow, styles.successText)}
            >
              <Icon icon={Check} size="sm" tone="success" />
              <span className={styles.smallText}>
                Installed: {item.installed_scopes.join(", ")}
              </span>
            </span>
          )}
        </div>

        <div className={styles.cardActionRow}>
          <Button
            size="sm"
            variant="primary"
            onClick={() => onInstall(item)}
            disabled={isInstalling}
            loading={isInstalling}
            className={styles.grow}
          >
            {isInstalling ? "Installing…" : "Install"}
          </Button>
          {item.homepage && (
            <Button
              size="sm"
              variant="ghost"
              rightIcon={ExternalLink}
              onClick={() =>
                window.open(item.homepage, "_blank", "noopener,noreferrer")
              }
            >
              Source
            </Button>
          )}
        </div>
      </div>
    </KitCard>
  );
};
