import React from "react";
import * as SwitchPrimitive from "@radix-ui/react-switch";
import classNames from "classnames";

import styles from "./Switch.module.css";

export interface SwitchProps extends SwitchPrimitive.SwitchProps {
  label?: React.ReactNode;
}

export const Switch = React.forwardRef<HTMLButtonElement, SwitchProps>(
  ({ className, label, ...props }, ref) => {
    const control = (
      <SwitchPrimitive.Root
        {...props}
        ref={ref}
        className={classNames(styles.root, className)}
      >
        <SwitchPrimitive.Thumb className={styles.thumb} />
      </SwitchPrimitive.Root>
    );

    if (!label) {
      return control;
    }

    return (
      <label className={styles.labelWrap}>
        {control}
        <span className={styles.label}>{label}</span>
      </label>
    );
  },
);
Switch.displayName = "Switch";
