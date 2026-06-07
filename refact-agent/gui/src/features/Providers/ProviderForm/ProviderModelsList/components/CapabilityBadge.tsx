import { Check, X } from "lucide-react";
import { type FC } from "react";

import { Badge, Icon } from "../../../../../components/ui";
import styles from "../ModelCard.module.css";

type CapabilityBadgeProps = {
  name: string;
  enabled: boolean;
  displayValue?: string | null;
  onClick?: () => void;
  interactive?: boolean;
};

export const CapabilityBadge: FC<CapabilityBadgeProps> = ({
  name,
  enabled,
  onClick,
  displayValue = null,
  interactive = true,
}) => {
  return (
    <Badge
      tone={enabled ? "success" : "muted"}
      onClick={interactive ? onClick : undefined}
      className={interactive ? styles.capabilityBadgeInteractive : undefined}
    >
      {name} {displayValue ? displayValue : <Icon icon={enabled ? Check : X} size="sm" />}
    </Badge>
  );
};
