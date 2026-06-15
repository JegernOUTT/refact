import React from "react";
import { ChevronDown } from "lucide-react";
import classNames from "classnames";
import { Icon } from "../ui";
import styles from "./Chevron.module.css";

export type ChevronProps = {
  open: boolean;
  className?: string;
  isUpDownChevron?: boolean;
};

export const Chevron: React.FC<ChevronProps> = ({
  open,
  className,
  isUpDownChevron = false,
}) => {
  return (
    <Icon
      icon={ChevronDown}
      size="sm"
      className={classNames(
        styles.chevron,
        {
          [styles.down]: open,
          [styles.right]: !open && !isUpDownChevron,
          [styles.up]: !open && isUpDownChevron,
        },
        className,
      )}
      aria-hidden
    />
  );
};
