import React from "react";
import classNames from "classnames";
import styles from "./ListRow.module.css";

export type ListRowElement = "div" | "button" | "a";
export type ListRowVariant = "plain" | "glass";

interface ListRowOwnProps {
  as?: ListRowElement;
  leading?: React.ReactNode;
  title: React.ReactNode;
  subtitle?: React.ReactNode;
  meta?: React.ReactNode;
  trailing?: React.ReactNode;
  variant?: ListRowVariant;
  selected?: boolean;
  interactive?: boolean;
  animated?: boolean;
  className?: string;
}

export type ListRowProps = ListRowOwnProps &
  Omit<React.HTMLAttributes<HTMLElement>, keyof ListRowOwnProps> & {
    href?: string;
    target?: string;
    rel?: string;
    type?: "button" | "submit" | "reset";
    disabled?: boolean;
  };

const variantClass: Record<ListRowVariant, string> = {
  plain: styles.plain,
  glass: styles.glass,
};

export function ListRow({
  as,
  leading,
  title,
  subtitle,
  meta,
  trailing,
  variant = "plain",
  selected = false,
  interactive,
  animated = false,
  className,
  ...props
}: ListRowProps) {
  const Component = as ?? "div";
  const isInteractive =
    interactive ??
    (Component === "button" ||
      Component === "a" ||
      typeof props.onClick === "function");
  const componentProps = {
    ...(Component === "button" ? { type: props.type ?? "button" } : null),
    ...props,
    className: classNames(
      styles.row,
      variantClass[variant],
      selected && styles.selected,
      isInteractive && styles.interactive,
      isInteractive && "rf-pressable",
      animated && "rf-enter-rise",
      className,
    ),
    "data-selected": selected ? "true" : undefined,
  };

  return React.createElement(
    Component,
    componentProps,
    leading ? <span className={styles.leading}>{leading}</span> : null,
    <span className={styles.content}>
      <span className={styles.title}>{title}</span>
      {subtitle ? <span className={styles.subtitle}>{subtitle}</span> : null}
    </span>,
    meta ? <span className={styles.meta}>{meta}</span> : null,
    trailing ? <span className={styles.trailing}>{trailing}</span> : null,
  );
}
