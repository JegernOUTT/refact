import React from "react";
import classNames from "classnames";
import { Surface } from "../Surface";
import styles from "./Card.module.css";

export interface CardProps extends React.ComponentPropsWithoutRef<"div"> {
  selected?: boolean;
}

export function Card({ selected = false, className, ...props }: CardProps) {
  return (
    <Surface
      className={classNames(styles.card, className)}
      radius="card"
      variant={selected ? "selected" : "surface-1"}
      {...props}
    />
  );
}
