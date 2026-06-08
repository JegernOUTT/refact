import React from "react";
import classNames from "classnames";
import styles from "./Badge.module.css";

export type BadgeTone =
  | "default"
  | "accent"
  | "success"
  | "warning"
  | "danger"
  | "muted";

export interface BadgeProps extends React.ComponentPropsWithoutRef<"span"> {
  tone?: BadgeTone;
  size?: string;
  variant?: string;
}

const toneClass: Record<BadgeTone, string> = {
  default: styles.default,
  accent: styles.accent,
  success: styles.success,
  warning: styles.warning,
  danger: styles.danger,
  muted: styles.muted,
};

export function Badge({
  tone = "default",
  className,
  size: _size,
  variant: _variant,
  ...props
}: BadgeProps) {
  return (
    <span
      className={classNames(styles.badge, toneClass[tone], className)}
      {...props}
    />
  );
}
