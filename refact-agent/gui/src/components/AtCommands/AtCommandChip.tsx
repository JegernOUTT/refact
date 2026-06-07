import React from "react";
import {
  File,
  Globe,
  HelpCircle,
  Library,
  ListTree,
  MapPin,
  Rows3,
  Search,
} from "lucide-react";
import { Icon } from "../ui";
import type { ChipDisplayInfo } from "../../utils/atCommands";
import styles from "./AtCommandChip.module.css";

type AtCommandChipProps = {
  chip: ChipDisplayInfo;
  onClick?: () => void;
};

const CHIP_ICONS: Record<ChipDisplayInfo["type"], React.ComponentProps<typeof Icon>["icon"]> = {
  file: File,
  web: Globe,
  tree: Rows3,
  search: Search,
  definition: MapPin,
  "knowledge-load": Library,
  references: ListTree,
  help: HelpCircle,
};

export const AtCommandChip: React.FC<AtCommandChipProps> = ({
  chip,
  onClick,
}) => {
  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (!chip.disabled && onClick) {
      onClick();
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if ((e.key === "Enter" || e.key === " ") && !chip.disabled && onClick) {
      e.preventDefault();
      e.stopPropagation();
      onClick();
    }
  };

  return (
    <span
      className={`${styles.chip} ${chip.disabled ? styles.disabled : ""}`}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      title={chip.fullPath ?? chip.label}
      role="button"
      tabIndex={chip.disabled ? -1 : 0}
      aria-disabled={chip.disabled}
    >
      <Icon icon={CHIP_ICONS[chip.type]} size="sm" className={styles.icon} />
      <span className={styles.label}>{chip.label}</span>
    </span>
  );
};
