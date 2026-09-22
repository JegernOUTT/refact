import React from "react";
import classNames from "classnames";
import styles from "./Text.module.css";

export interface TextProps extends React.ComponentPropsWithoutRef<"span"> {
  as?: "span" | "p" | "div" | "small" | "strong" | "h1" | "h2" | "h3" | "h4";
  size?: "1" | "2" | "3" | "4" | "5";
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
  const heading = as === "h1" || as === "h2" || as === "h3" || as === "h4";
  return (
    <Component
      {...props}
      className={classNames(
        styles.text,
        heading && styles.heading,
        styles[`size-${size}`],
        styles[`weight-${weight}`],
        color && styles[`color-`],
        align && styles[`align-`],
        className,
      )}
    />
  );
}
