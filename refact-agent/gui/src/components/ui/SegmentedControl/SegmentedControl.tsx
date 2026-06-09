import React from "react";
import classNames from "classnames";

import { Icon } from "../Icon";
import styles from "./SegmentedControl.module.css";

export interface SegmentedControlOption {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
  iconOnly?: boolean;
  ariaLabel?: string;
}

export interface SegmentedControlProps
  extends Omit<React.ComponentProps<"div">, "onChange"> {
  options: SegmentedControlOption[];
  value: string;
  onValueChange: (value: string) => void;
  size?: "sm" | "md";
  name?: string;
}

function isIconOnlyLabel(label: React.ReactNode): boolean {
  const children = React.Children.toArray(label);

  if (children.length !== 1) return false;

  const child = children[0];

  if (!React.isValidElement(child)) return false;

  return child.type === "svg" || child.type === Icon;
}

export function SegmentedControl({
  className,
  name,
  onValueChange,
  options,
  size = "md",
  style,
  value,
  ...props
}: SegmentedControlProps) {
  const hasOptions = options.length > 0;
  const activeIndex = hasOptions
    ? Math.max(
        0,
        options.findIndex((option) => option.value === value),
      )
    : 0;

  return (
    <div
      {...props}
      aria-disabled={hasOptions ? props["aria-disabled"] : true}
      className={classNames(styles.root, styles[`size-${size}`], className)}
      role="radiogroup"
      style={
        {
          ...style,
          "--rf-segment-count": hasOptions ? options.length : 1,
          "--rf-segment-index": activeIndex,
        } as React.CSSProperties
      }
    >
      {hasOptions ? (
        <span aria-hidden="true" className={styles.indicator} />
      ) : null}
      {options.map((option) => {
        const iconOnly = option.iconOnly ?? isIconOnlyLabel(option.label);

        return (
          <label key={option.value} className={styles.segment}>
            <input
              aria-label={
                option.ariaLabel ?? (iconOnly ? option.value : undefined)
              }
              checked={option.value === value}
              className={styles.input}
              disabled={option.disabled}
              name={name}
              type="radio"
              value={option.value}
              onChange={() => onValueChange(option.value)}
            />
            <span
              className={classNames(styles.label, {
                [styles.labelIconOnly]: iconOnly,
              })}
            >
              <span className={styles.content}>{option.label}</span>
            </span>
          </label>
        );
      })}
    </div>
  );
}
