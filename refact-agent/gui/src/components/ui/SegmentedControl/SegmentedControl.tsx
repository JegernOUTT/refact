import React from "react";
import classNames from "classnames";

import styles from "./SegmentedControl.module.css";

export interface SegmentedControlOption {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
}

export interface SegmentedControlProps
  extends Omit<React.ComponentProps<"div">, "onChange"> {
  options: SegmentedControlOption[];
  value: string;
  onValueChange: (value: string) => void;
  size?: "sm" | "md";
  name?: string;
}

export function SegmentedControl({
  className,
  name,
  onValueChange,
  options,
  size = "md",
  value,
  ...props
}: SegmentedControlProps) {
  const activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );

  return (
    <div
      {...props}
      className={classNames(styles.root, styles[`size-${size}`], className)}
      role="radiogroup"
      style={
        {
          "--rf-segment-count": options.length,
          "--rf-segment-index": activeIndex,
        } as React.CSSProperties
      }
    >
      <span aria-hidden="true" className={styles.indicator} />
      {options.map((option) => (
        <label key={option.value} className={styles.segment}>
          <input
            checked={option.value === value}
            className={styles.input}
            disabled={option.disabled}
            name={name}
            type="radio"
            value={option.value}
            onChange={() => onValueChange(option.value)}
          />
          <span className={styles.label}>{option.label}</span>
        </label>
      ))}
    </div>
  );
}
