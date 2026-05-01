import React from "react";
import { Badge, Flex } from "@radix-ui/themes";
import {
  ExclamationTriangleIcon,
  GearIcon,
  IdCardIcon,
} from "@radix-ui/react-icons";
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
    <Flex gap="2" wrap="wrap" align="center">
      <Badge
        color={selectedSource === null ? "blue" : "gray"}
        variant={selectedSource === null ? "solid" : "soft"}
        className={styles.sourceTab}
        onClick={() => onSelectSource(null)}
      >
        All ({totalCount})
      </Badge>
      {sources.map((source) => (
        <Badge
          key={source.id}
          color={
            source.status === "error"
              ? "red"
              : !source.enabled
                ? "gray"
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
          {source.server_count !== undefined && ` (${source.server_count})`}
          {source.status === "error" && <ExclamationTriangleIcon />}
          {source.needs_api_key && !source.has_api_key && <IdCardIcon />}
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
