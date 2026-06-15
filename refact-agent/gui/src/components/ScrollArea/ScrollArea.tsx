import classNames from "classnames";
import React from "react";
import styles from "./ScrollArea.module.css";

export type ScrollAreaProps = React.ComponentPropsWithoutRef<"div"> & {
  asChild?: boolean;
  className?: string;
  scrollbars?: "vertical" | "horizontal" | "both" | undefined;
  fullHeight?: boolean;
  type?: "auto" | "always" | "scroll" | "hover";
};
export const ScrollArea = React.forwardRef<HTMLDivElement, ScrollAreaProps>(
  (
    {
      asChild,
      children,
      className,
      fullHeight,
      scrollbars,
      type: _type,
      ...props
    },
    ref,
  ) => {
    const rootClassName = classNames(
      styles.root,
      scrollbars === "vertical" && styles.vertical,
      scrollbars === "horizontal" && styles.horizontal,
      fullHeight && styles.full_height,
      className,
    );

    if (asChild && React.isValidElement(children)) {
      const child = children as React.ReactElement<
        React.HTMLAttributes<HTMLDivElement>
      >;
      const childProps = child.props;
      return React.cloneElement(child, {
        ...props,
        className: classNames(
          rootClassName,
          styles.viewport,
          childProps.className,
        ),
      });
    }

    return (
      <div className={rootClassName}>
        <div {...props} ref={ref} className={styles.viewport}>
          {children}
        </div>
      </div>
    );
  },
);

ScrollArea.displayName = "ScrollArea";
