import React from "react";
import classNames from "classnames";
import styles from "./Spinner.module.css";

export interface SpinnerProps {
  size?: "sm" | "md" | "lg";
  label?: string;
}

export const Spinner: React.FC<SpinnerProps> = ({ size = "md", label = "Loading" }) => (
  <pre
    role="status"
    aria-label={label}
    className={classNames(styles.spinner, styles.spinning, styles[size])}
  />
);
