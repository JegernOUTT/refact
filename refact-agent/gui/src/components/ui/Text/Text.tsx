import React from "react";
import classNames from "classnames";
import styles from "./Text.module.css";

export interface TextProps extends React.ComponentPropsWithoutRef<"span"> {
  as?: "span" | "p" | "div" | "small" | "strong";
  size?: "1" | "2" | "3" | "4";
  weight?: "regular" | "medium" | "bold";
  color?: "gray" | "red" | "orange" | "accent";
  align?: "left" | "center" | "right";
}

export function Text({
  as = "span",
  align,
  className,
  color,
  size = "2",
  weight = "regular",
  ...props
}: TextProps) {
  const Component = as;
  return (
    <Component
      {...props}
      className={classNames(
        styles.text,
        styles[`size-${size}`],
        styles[`weight-${weight}`],
        color && styles[`color-`],
        align && styles[`align-`],
        className,
      )}
    />
  );
}
