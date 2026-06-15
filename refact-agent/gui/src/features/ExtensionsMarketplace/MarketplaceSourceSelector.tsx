import React from "react";
import { AlertTriangle, Settings } from "lucide-react";
import { Badge, Icon } from "../../components/ui";
import type { ExtensionMarketplaceSource } from "../../services/refact/extensionsMarketplace";
import styles from "./ExtensionsMarketplace.module.css";

type MarketplaceSourceSelectorProps = {
  sources: ExtensionMarketplaceSource[];
  selectedSource: string | null;
  onSelectSource: (sourceId: string | null) => void;
  onOpenSettings: () => void;
};

export const MarketplaceSourceSelector: React.FC<
  MarketplaceSourceSelectorProps
> = ({ sources, selectedSource, onSelectSource, onOpenSettings }) => {
  const total = sources.reduce(
    (acc, source) => acc + (source.item_count ?? 0),
    0,
  );

  return (
    <div className={styles.sourceSelector}>
      <Badge
        tone={selectedSource === null ? "accent" : "muted"}
        className={styles.sourceTab}
        role="button"
        tabIndex={0}
        onClick={() => onSelectSource(null)}
      >
        All ({total})
      </Badge>
      {sources.map((source) => (
        <Badge
          key={source.id}
          tone={
            source.error
              ? "danger"
              : selectedSource === source.id
                ? "accent"
                : "muted"
          }
          className={
            source.enabled ? styles.sourceTab : styles.sourceTabDisabled
          }
          role="button"
          tabIndex={source.enabled ? 0 : -1}
          onClick={() =>
            source.enabled &&
            onSelectSource(selectedSource === source.id ? null : source.id)
          }
        >
          {source.label}
          {source.item_count !== undefined && ` (${source.item_count})`}
          {source.error && (
            <Icon icon={AlertTriangle} size="sm" tone="danger" />
          )}
        </Badge>
      ))}
      <Badge
        tone="muted"
        className={styles.sourceTab}
        role="button"
        tabIndex={0}
        onClick={onOpenSettings}
        title="Manage marketplace sources"
      >
        <Icon icon={Settings} size="sm" tone="muted" />
      </Badge>
    </div>
  );
};
