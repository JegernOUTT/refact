import React from "react";
import classNames from "classnames";
import styles from "./Surface.module.css";

export type SurfaceVariant =
  | "plain"
  | "surface-1"
  | "surface-2"
  | "surface-3"
  | "overlay"
  | "selected";

export type SurfaceRadius = "none" | "chip" | "control" | "card" | "pill";

type SurfaceOwnProps<T extends React.ElementType> = {
  as?: T;
  variant?: SurfaceVariant;
  radius?: SurfaceRadius;
};

export type SurfaceProps<T extends React.ElementType = "div"> = SurfaceOwnProps<T> &
  Omit<React.ComponentPropsWithoutRef<T>, keyof SurfaceOwnProps<T>>;

const variantClass: Record<SurfaceVariant, string> = {
  plain: styles.plain,
  "surface-1": styles.surface1,
  "surface-2": styles.surface2,
  "surface-3": styles.surface3,
  overlay: styles.overlay,
  selected: styles.selected,
};

const radiusClass: Record<SurfaceRadius, string> = {
  none: styles.radiusNone,
  chip: styles.radiusChip,
  control: styles.radiusControl,
  card: styles.radiusCard,
  pill: styles.radiusPill,
};

export function Surface<T extends React.ElementType = "div">({
  as,
  variant = "plain",
  radius = "card",
  className,
  ...props
}: SurfaceProps<T>) {
  const Component = as ?? "div";

  return (
    <Component
      className={classNames(
        styles.surface,
        variantClass[variant],
        radiusClass[radius],
        className,
      )}
      {...props}
    />
  );
}
