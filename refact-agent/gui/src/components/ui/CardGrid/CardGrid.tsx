import React from "react";
import classNames from "classnames";

import styles from "./CardGrid.module.css";

export interface CardGridProps extends React.ComponentPropsWithoutRef<"div"> {
  /** Narrower track minimum (240px instead of the default 280px). */
  dense?: boolean;
}

/**
 * Presentational responsive card grid.
 * Kit-only: no feature imports, no inline styles, tokens for spacing.
 */
export const CardGrid = React.forwardRef<HTMLDivElement, CardGridProps>(
  function CardGrid({ dense = false, className, ...props }, ref) {
    return (
      <div
        ref={ref}
        className={classNames(
          styles.grid,
          dense ? styles.dense : styles.regular,
          className,
        )}
        {...props}
      />
    );
  },
);
