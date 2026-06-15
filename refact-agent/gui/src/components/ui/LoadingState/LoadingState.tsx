import React from "react";
import classNames from "classnames";
import { Skeleton, SkeletonText } from "../Skeleton";
import { Spinner } from "../Spinner";
import styles from "./LoadingState.module.css";

export type LoadingStateVariant = "compact" | "full";
export type LoadingStateKind = "spinner" | "skeleton";

export interface LoadingStateProps
  extends React.ComponentPropsWithoutRef<"section"> {
  label?: React.ReactNode;
  variant?: LoadingStateVariant;
  kind?: LoadingStateKind;
}

export function LoadingState({
  label = "Loading",
  variant = "compact",
  kind = "spinner",
  className,
  ...props
}: LoadingStateProps) {
  return (
    <section
      aria-busy="true"
      className={classNames(styles.loadingState, styles[variant], className)}
      {...props}
    >
      {kind === "skeleton" ? (
        <div className={styles.skeletonStack}>
          <Skeleton
            height={variant === "full" ? "88px" : "48px"}
            radius="card"
          />
          <SkeletonText lines={variant === "full" ? 4 : 2} />
        </div>
      ) : (
        <Spinner label={typeof label === "string" ? label : "Loading"} />
      )}
      {label ? <p className={styles.label}>{label}</p> : null}
    </section>
  );
}
