import React from "react";
import classNames from "classnames";
import { Badge } from "../Badge";
import { Icon } from "../Icon";
import {
  getStatusBadgeRecipe,
  type StatusBadgeProps,
  type StatusBadgeSize,
} from "./statusBadgeRecipe";
import styles from "./StatusBadge.module.css";

const iconSize: Record<StatusBadgeSize, React.ComponentProps<typeof Icon>["size"]> = {
  xs: "sm",
  sm: "sm",
  md: "md",
};

export function StatusBadge({
  status,
  tone,
  label,
  ariaLabel,
  icon,
  size = "sm",
  variant = "soft",
  pulse,
  className,
  ...props
}: StatusBadgeProps) {
  const recipe = getStatusBadgeRecipe(status);
  const visibleLabel = label ?? recipe.label;
  const accessibleLabel = ariaLabel ?? label ?? recipe.ariaLabel;
  const shouldPulse =
    status === "running" && (pulse === true || recipe.pulse === true);

  return (
    <Badge
      aria-label={accessibleLabel}
      className={classNames(
        styles.statusBadge,
        shouldPulse && styles.pulse,
        className,
      )}
      data-status={status}
      data-tone={tone ?? recipe.tone}
      size={size}
      tone={tone ?? recipe.tone}
      variant={variant}
      {...props}
    >
      {icon ? (
        <Icon className={styles.icon} icon={icon} size={iconSize[size]} />
      ) : null}
      <span className={styles.label}>{visibleLabel}</span>
    </Badge>
  );
}
