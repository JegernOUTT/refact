import React from "react";
import classNames from "classnames";
import { X } from "lucide-react";
import { Icon } from "../Icon";
import styles from "./Chip.module.css";

export interface ChipProps extends React.ComponentPropsWithoutRef<"span"> {
  icon?: React.ReactNode;
  removable?: boolean;
  selected?: boolean;
  disabled?: boolean;
  onRemove?: () => void;
  radius?: "chip" | "pill";
}

export function Chip({
  icon,
  removable = false,
  selected = false,
  disabled = false,
  onRemove,
  radius = "pill",
  className,
  children,
  ...props
}: ChipProps) {
  return (
    <span
      className={classNames(
        styles.chip,
        selected && styles.selected,
        disabled && styles.disabled,
        radius === "pill" ? styles.radiusPill : styles.radiusChip,
        className,
      )}
      aria-disabled={disabled || undefined}
      {...props}
    >
      {icon ? <span className={styles.icon}>{icon}</span> : null}
      <span className={styles.label}>{children}</span>
      {removable ? (
        <button
          aria-label="Remove"
          className={styles.remove}
          disabled={disabled}
          onClick={onRemove}
          type="button"
        >
          <Icon icon={X} size="md" />
        </button>
      ) : null}
    </span>
  );
}
