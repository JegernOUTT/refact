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
export type BadgeSize = "xs" | "sm" | "md";
export type BadgeVariant = "soft" | "outline" | "glass";

export interface BadgeProps
  extends Omit<React.ComponentPropsWithoutRef<"span">, "size"> {
  tone?: BadgeTone;
  size?: BadgeSize;
  variant?: BadgeVariant;
  interactive?: boolean;
}

type BadgeComponentProps = Omit<BadgeProps, "size" | "variant"> & {
  size?: BadgeSize | "1" | "2" | "3" | (string & Record<never, never>);
  variant?: BadgeVariant | (string & Record<never, never>);
};

const toneClass: Record<BadgeTone, string> = {
  default: styles.default,
  accent: styles.accent,
  success: styles.success,
  warning: styles.warning,
  danger: styles.danger,
  muted: styles.muted,
};

const sizeClass: Record<BadgeSize, string> = {
  xs: styles["size-xs"],
  sm: styles["size-sm"],
  md: styles["size-md"],
};

const variantClass: Record<BadgeVariant, string> = {
  soft: styles["variant-soft"],
  outline: styles["variant-outline"],
  glass: styles["variant-glass"],
};

function normalizeSize(size: BadgeComponentProps["size"]): BadgeSize {
  if (size === "xs" || size === "1") return "xs";
  if (size === "md" || size === "3") return "md";
  return "sm";
}

function normalizeVariant(
  variant: BadgeComponentProps["variant"],
): BadgeVariant {
  if (variant === "outline") return "outline";
  if (variant === "glass") return "glass";
  return "soft";
}

export const Badge = React.forwardRef<HTMLSpanElement, BadgeComponentProps>(
  (
    {
      tone = "default",
      size = "sm",
      variant = "soft",
      interactive = false,
      className,
      ...props
    },
    ref,
  ) => {
    const normalizedSize = normalizeSize(size);
    const normalizedVariant = normalizeVariant(variant);

    return (
      <span
        ref={ref}
        className={classNames(
          styles.badge,
          toneClass[tone],
          sizeClass[normalizedSize],
          variantClass[normalizedVariant],
          interactive && styles.interactive,
          className,
        )}
        {...props}
      />
    );
  },
);

Badge.displayName = "Badge";
