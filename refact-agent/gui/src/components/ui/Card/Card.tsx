import React from "react";
import classNames from "classnames";
import { Surface, type SurfaceAnimation } from "../Surface";
import styles from "./Card.module.css";

export interface CardProps extends React.ComponentPropsWithoutRef<"div"> {
  selected?: boolean;
  animated?: SurfaceAnimation;
  interactive?: boolean;
}

export function Card({
  selected = false,
  animated = false,
  interactive,
  className,
  ...props
}: CardProps) {
  return (
    <Surface
      animated={animated}
      className={classNames(styles.card, className)}
      interactive={interactive}
      radius="card"
      variant={selected ? "selected" : "surface-1"}
      {...props}
    />
  );
}
