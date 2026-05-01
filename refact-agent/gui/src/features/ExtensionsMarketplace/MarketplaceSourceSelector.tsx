import React from "react";
import { Badge, Flex } from "@radix-ui/themes";
import { ExclamationTriangleIcon, GearIcon } from "@radix-ui/react-icons";
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
    <Flex gap="2" wrap="wrap" align="center">
      <Badge
        color={selectedSource === null ? "blue" : "gray"}
        variant={selectedSource === null ? "solid" : "soft"}
        className={styles.sourceTab}
        onClick={() => onSelectSource(null)}
      >
        All ({total})
      </Badge>
      {sources.map((source) => (
        <Badge
          key={source.id}
          color={
            source.error
              ? "red"
              : selectedSource === source.id
                ? "blue"
                : "gray"
          }
          variant={selectedSource === source.id ? "solid" : "soft"}
          className={styles.sourceTab}
          onClick={() =>
            source.enabled &&
            onSelectSource(selectedSource === source.id ? null : source.id)
          }
          style={{ opacity: source.enabled ? 1 : 0.5 }}
        >
          {source.label}
          {source.item_count !== undefined && ` (${source.item_count})`}
          {source.error && <ExclamationTriangleIcon />}
        </Badge>
      ))}
      <Badge
        color="gray"
        variant="soft"
        className={styles.sourceTab}
        onClick={onOpenSettings}
        title="Manage marketplace sources"
      >
        <GearIcon />
      </Badge>
    </Flex>
  );
};
