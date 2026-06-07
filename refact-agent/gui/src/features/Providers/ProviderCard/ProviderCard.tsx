import React from "react";
import { Copy } from "lucide-react";

import { Badge, IconButton, StatusDot, Surface, Tooltip } from "../../../components/ui";
import { getProviderIcon } from "../icons/iconsMap";
import type { ProviderListItem, ProviderStatus } from "../../../services/refact";
import { getProviderName } from "../getProviderName";

import styles from "./ProviderCard.module.css";

export type ProviderCardProps = {
  provider: ProviderListItem;
  setCurrentProvider: (provider: ProviderListItem) => void;
  onDuplicateProvider?: (provider: ProviderListItem) => void;
};

function statusTone(status: ProviderStatus): React.ComponentProps<typeof StatusDot>["status"] {
  if (status === "active") return "success";
  if (status === "configured") return "warning";
  return "idle";
}

export const ProviderCard: React.FC<ProviderCardProps> = ({
  provider,
  setCurrentProvider,
  onDuplicateProvider,
}) => {
  const providerName = getProviderName(provider);
  const showInstanceId =
    provider.name !== provider.display_name || provider.base_provider !== provider.name;
  const handleDuplicateClick = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    onDuplicateProvider?.(provider);
  };

  return (
    <Surface
      as="button"
      type="button"
      variant="plain"
      onClick={() => setCurrentProvider(provider)}
      className={styles.providerCard}
    >
      <span className={styles.identity}>
        <span className={styles.iconWrap}>{getProviderIcon(provider)}</span>
        <span className={styles.copy}>
          <span className={styles.providerName} role="heading" aria-level={3}>
            {providerName}
          </span>
          {showInstanceId ? <span className={styles.providerId}>{provider.name}</span> : null}
        </span>
      </span>
      <span className={styles.meta}>
        {onDuplicateProvider ? (
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                type="button"
                size="sm"
                variant="ghost"
                aria-label={`Duplicate ${providerName}`}
                icon={Copy}
                onClick={handleDuplicateClick}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Duplicate instance</Tooltip.Content>
          </Tooltip>
        ) : null}
        {provider.model_count > 0 ? (
          <Badge tone="muted">
            {provider.model_count} model{provider.model_count !== 1 ? "s" : ""}
          </Badge>
        ) : null}
        <StatusDot status={statusTone(provider.status)} pulse={provider.status === "active"} />
      </span>
    </Surface>
  );
};
