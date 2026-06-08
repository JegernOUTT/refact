import React from "react";
import classNames from "classnames";
import styles from "./Skeleton.module.css";

export type SkeletonRadius = "none" | "chip" | "control" | "card" | "pill";

export interface SkeletonProps extends React.ComponentPropsWithoutRef<"div"> {
  width?: string;
  height?: string;
  radius?: SkeletonRadius;
}

export interface SkeletonTextProps
  extends React.ComponentPropsWithoutRef<"div"> {
  lines?: number;
  radius?: SkeletonRadius;
}

const radiusClass: Record<SkeletonRadius, string> = {
  none: styles.radiusNone,
  chip: styles.radiusChip,
  control: styles.radiusControl,
  card: styles.radiusCard,
  pill: styles.radiusPill,
};

function skeletonStyle(
  width?: string,
  height?: string,
): React.CSSProperties | undefined {
  if (!width && !height) return undefined;
  return {
    ...(width ? { "--skeleton-width": width } : {}),
    ...(height ? { "--skeleton-height": height } : {}),
  } as React.CSSProperties;
}

export function Skeleton({
  width,
  height,
  radius = "control",
  className,
  style,
  ...props
}: SkeletonProps) {
  return (
    <div
      aria-hidden="true"
      className={classNames(
        "rf-shimmer",
        styles.skeleton,
        radiusClass[radius],
        className,
      )}
      style={{ ...skeletonStyle(width, height), ...style }}
      {...props}
    />
  );
}

export function SkeletonText({
  lines = 3,
  radius = "control",
  className,
  ...props
}: SkeletonTextProps) {
  return (
    <div className={classNames(styles.text, className)} {...props}>
      {Array.from({ length: lines }).map((_, index) => (
        <Skeleton
          className={styles.textLine}
          key={index}
          radius={radius}
          width={index === lines - 1 ? "72%" : "100%"}
        />
      ))}
    </div>
  );
}
