import React from "react";
import { Text } from "@radix-ui/themes";
import {
  FileIcon,
  GlobeIcon,
  MagnifyingGlassIcon,
  QuestionMarkCircledIcon,
  ReaderIcon,
  SewingPinIcon,
  RowsIcon,
} from "@radix-ui/react-icons";
import type { ChipDisplayInfo } from "../../utils/atCommands";
import styles from "./AtCommandChip.module.css";

type AtCommandChipProps = {
  chip: ChipDisplayInfo;
  onClick?: () => void;
};

const CHIP_ICONS: Record<ChipDisplayInfo["type"], React.ReactNode> = {
  file: <FileIcon />,
  web: <GlobeIcon />,
  tree: <RowsIcon />,
  search: <MagnifyingGlassIcon />,
  definition: <SewingPinIcon />,
  "knowledge-load": <ReaderIcon />,
  references: <SewingPinIcon />,
  help: <QuestionMarkCircledIcon />,
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
      <span className={styles.icon}>{CHIP_ICONS[chip.type]}</span>
      <Text size="1" className={styles.label}>
        {chip.label}
      </Text>
    </span>
  );
};
