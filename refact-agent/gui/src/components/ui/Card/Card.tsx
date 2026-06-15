import React from "react";
import classNames from "classnames";
import {
  Surface,
  type SurfaceAnimation,
  type SurfaceVariant,
} from "../Surface";
import styles from "./Card.module.css";

export type CardPadding = "none" | "sm" | "md" | "lg";

export interface CardProps extends React.ComponentPropsWithoutRef<"div"> {
  selected?: boolean;
  variant?: SurfaceVariant;
  animated?: SurfaceAnimation;
  interactive?: boolean;
  padding?: CardPadding;
}

const paddingClass: Record<CardPadding, string> = {
  none: styles.paddingNone,
  sm: styles.paddingSm,
  md: styles.paddingMd,
  lg: styles.paddingLg,
};

export function Card({
  selected = false,
  variant = "surface-1",
  animated = false,
  interactive,
  padding = "md",
  className,
  ...props
}: CardProps) {
  return (
    <Surface
      animated={animated}
      className={classNames(styles.card, paddingClass[padding], className)}
      data-card-variant={variant}
      data-selected={selected ? "true" : undefined}
      interactive={interactive}
      radius="card"
      variant={variant}
      {...props}
    />
  );
}

export type GlassCardProps = Omit<CardProps, "variant">;

export function GlassCard(props: GlassCardProps) {
  return <Card {...props} variant="glass" />;
}
