import React from "react";
import { AlertTriangle, IdCard, Settings } from "lucide-react";
import { Badge, Icon } from "../../components/ui";
import type { MarketplaceSource } from "../../services/refact/mcpMarketplace";
import styles from "./MCPMarketplace.module.css";

type SourceSelectorProps = {
  sources: MarketplaceSource[];
  selectedSource: string | null;
  onSelectSource: (sourceId: string | null) => void;
  onOpenSettings: () => void;
};

export const SourceSelector: React.FC<SourceSelectorProps> = ({
  sources,
  selectedSource,
  onSelectSource,
  onOpenSettings,
}) => {
  const totalCount = sources.reduce((acc, s) => acc + (s.server_count ?? 0), 0);

  return (
    <div className={styles.sourceSelector}>
      <Badge
        tone={selectedSource === null ? "accent" : "muted"}
        className={styles.sourceTab}
        role="button"
        tabIndex={0}
        onClick={() => onSelectSource(null)}
      >
        All ({totalCount})
      </Badge>
      {sources.map((source) => (
        <Badge
          key={source.id}
          tone={
            source.status === "error"
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
          {source.server_count !== undefined && ` (${source.server_count})`}
          {source.status === "error" && <Icon icon={AlertTriangle} size="sm" tone="danger" />}
          {source.needs_api_key && !source.has_api_key && <Icon icon={IdCard} size="sm" tone="warning" />}
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
