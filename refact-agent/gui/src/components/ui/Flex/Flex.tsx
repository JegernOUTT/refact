import React from "react";
import classNames from "classnames";
import styles from "./Flex.module.css";

export interface FlexProps extends React.ComponentPropsWithoutRef<"div"> {
  align?: "start" | "center" | "end" | "stretch";
  direction?: "row" | "column";
  gap?: "0" | "1" | "2" | "3" | "4";
  justify?: "start" | "center" | "end" | "between";
  py?: "1" | "2" | "3" | "4";
  wrap?: "nowrap" | "wrap";
}

export function Flex({
  align,
  className,
  direction = "row",
  gap,
  justify,
  py,
  wrap,
  ...props
}: FlexProps) {
  return (
    <div
      {...props}
      className={classNames(
        styles.flex,
        styles[`direction-${direction}`],
        align && styles[`align-${align}`],
        gap && styles[`gap-${gap}`],
        justify && styles[`justify-${justify}`],
        py && styles[`py-${py}`],
        wrap && styles[`wrap-${wrap}`],
        className,
      )}
    />
  );
}
