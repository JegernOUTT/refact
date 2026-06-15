import React from "react";
import { Copy } from "lucide-react";

import {
  Badge,
  IconButton,
  StatusDot,
  Surface,
  Tooltip,
} from "../../../components/ui";
import { getProviderIcon } from "../icons/iconsMap";
import type {
  ProviderListItem,
  ProviderStatus,
} from "../../../services/refact";
import { getProviderName } from "../getProviderName";

import styles from "./ProviderCard.module.css";

export type ProviderCardProps = {
  provider: ProviderListItem;
  setCurrentProvider: (provider: ProviderListItem) => void;
  onDuplicateProvider?: (provider: ProviderListItem) => void;
};

function statusTone(
  status: ProviderStatus,
): React.ComponentProps<typeof StatusDot>["status"] {
  if (status === "active") return "success";
  if (status === "configured") return "warning";
  return "idle";
}

function statusLabel(status: ProviderStatus) {
  if (status === "active") return "Active";
  if (status === "configured") return "Configured";
  return "Not configured";
}

export const ProviderCard: React.FC<ProviderCardProps> = ({
  provider,
  setCurrentProvider,
  onDuplicateProvider,
}) => {
  const providerName = getProviderName(provider);
  const showInstanceId =
    provider.name !== provider.display_name ||
    provider.base_provider !== provider.name;
  const handleSelectProvider = () => setCurrentProvider(provider);
  const handleDuplicateClick = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    onDuplicateProvider?.(provider);
  };
  const handleCardKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    handleSelectProvider();
  };

  return (
    <Surface
      as="div"
      role="button"
      tabIndex={0}
      variant="glass"
      animated="rise"
      interactive
      onClick={handleSelectProvider}
      onKeyDown={handleCardKeyDown}
      className={styles.providerCard}
    >
      <span className={styles.identity}>
        <span className={styles.iconWrap}>{getProviderIcon(provider)}</span>
        <span className={styles.copy}>
          <span className={styles.providerName} role="heading" aria-level={3}>
            {providerName}
          </span>
          {showInstanceId ? (
            <span className={styles.providerId}>{provider.name}</span>
          ) : null}
        </span>
      </span>
      <span className={styles.meta}>
        <span className={styles.modelBadgeWrap}>
          {provider.model_count > 0 ? (
            <Badge tone="muted" className={styles.modelBadge}>
              {provider.model_count} model
              {provider.model_count !== 1 ? "s" : ""}
            </Badge>
          ) : null}
        </span>
        <span
          className={styles.status}
          aria-label={statusLabel(provider.status)}
        >
          <StatusDot
            status={statusTone(provider.status)}
            pulse={provider.status === "active"}
          />
        </span>
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
      </span>
    </Surface>
  );
};
