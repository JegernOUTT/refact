import React from "react";
import * as SliderPrimitive from "@radix-ui/react-slider";
import classNames from "classnames";

import styles from "./Slider.module.css";

export interface SliderProps extends SliderPrimitive.SliderProps {
  label?: React.ReactNode;
  valueLabel?: React.ReactNode;
}

export const Slider = React.forwardRef<HTMLSpanElement, SliderProps>(
  ({ className, label, valueLabel, ...props }, ref) => {
    const control = (
      <SliderPrimitive.Root
        {...props}
        ref={ref}
        className={classNames(styles.root, className)}
      >
        <SliderPrimitive.Track className={styles.track}>
          <SliderPrimitive.Range className={styles.range} />
        </SliderPrimitive.Track>
        {(props.value ?? props.defaultValue ?? [0]).map((_, index) => (
          <SliderPrimitive.Thumb
            key={index}
            className={styles.thumb}
            aria-label={props["aria-label"] ?? (typeof label === "string" ? label : "Slider value")}
          />
        ))}
      </SliderPrimitive.Root>
    );

    if (!label && !valueLabel) {
      return control;
    }

    return (
      <label className={styles.field}>
        <span className={styles.header}>
          <span>{label}</span>
          {valueLabel ? <span className={styles.value}>{valueLabel}</span> : null}
        </span>
        {control}
      </label>
    );
  },
);
Slider.displayName = "Slider";
