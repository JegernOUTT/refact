import React from "react";
import classNames from "classnames";
import {
  STATUS_DOT_STATUS_TONE,
  type StatusDotStatus,
  type StatusDotTone,
} from "./statusTone";
import styles from "./StatusDot.module.css";

export interface StatusDotProps extends React.ComponentPropsWithoutRef<"span"> {
  status?: StatusDotStatus;
  size?: "small" | "medium" | "large";
  pulse?: boolean;
}

const toneClass: Record<StatusDotTone, string> = {
  muted: styles.muted,
  accent: styles.accent,
  success: styles.success,
  warning: styles.warning,
  danger: styles.danger,
};

const sizeClass: Record<NonNullable<StatusDotProps["size"]>, string> = {
  small: styles.small,
  medium: styles.medium,
  large: styles.large,
};

export function StatusDot({
  status = "idle",
  size = "small",
  pulse = false,
  className,
  ...props
}: StatusDotProps) {
  const tone = STATUS_DOT_STATUS_TONE[status];

  return (
    <span
      className={classNames(
        styles.dot,
        toneClass[tone],
        sizeClass[size],
        pulse && "rf-status-pulse",
        className,
      )}
      {...props}
    />
  );
}
