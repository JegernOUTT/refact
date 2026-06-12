import React from "react";
import { Badge, Tooltip } from "@radix-ui/themes";
import styles from "./ChatContent.module.css";
import type { SurgeryBadgeInfo } from "./SurgeryBadgeInfo";

export const SurgeryBadge: React.FC<{ info: SurgeryBadgeInfo | null }> = ({
  info,
}) => {
  if (!info) return null;
  return (
    <Tooltip content={info.detail}>
      <Badge
        size="1"
        variant="soft"
        color="purple"
        className={styles.surgeryBadge}
        data-testid="buddy-surgery-badge"
      >
        🩹 {info.label}
      </Badge>
    </Tooltip>
  );
};
