import React from "react";
import classNames from "classnames";
import { LoaderCircle } from "lucide-react";
import { Icon } from "../Icon";
import { Skeleton, SkeletonText } from "../Skeleton";
import styles from "./LoadingState.module.css";

export type LoadingStateVariant = "compact" | "full";
export type LoadingStateKind = "spinner" | "skeleton";

export interface LoadingStateProps
  extends React.ComponentPropsWithoutRef<"section"> {
  label?: React.ReactNode;
  variant?: LoadingStateVariant;
  kind?: LoadingStateKind;
}

// Compact tiles used to fall back to the text spinner, which collapses to a
// couple of pixels between animation frames. Both variants now render a real
// kit icon: var(--rf-icon) (15px) compact, var(--rf-icon-lg) (18px) full.
const spinnerSize: Record<LoadingStateVariant, "md" | "lg"> = {
  compact: "md",
  full: "lg",
};

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
        <span
          role="status"
          aria-label={typeof label === "string" ? label : "Loading"}
        >
          <Icon
            className="rf-spin"
            icon={LoaderCircle}
            size={spinnerSize[variant]}
            tone="accent"
          />
        </span>
      )}
      {label ? <p className={styles.label}>{label}</p> : null}
    </section>
  );
}
