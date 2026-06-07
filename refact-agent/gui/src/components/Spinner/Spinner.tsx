import React from "react";
import classNames from "classnames";
import styles from "./Spinner.module.css";

export type SpinnerProps = {
  spinning: boolean;
};

export const Spinner: React.FC<SpinnerProps> = ({ spinning }) => (
  <pre
    aria-busy={spinning}
    className={classNames(styles.spinner, spinning && styles.spinning)}
  />
);
