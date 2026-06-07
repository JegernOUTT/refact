import React from "react";
import classNames from "classnames";
import { Tooltip } from "../ui";
import styles from "./CircularProgress.module.css";

export interface CircularProgressProps {
  done: number;
  total: number;
  failed?: number;
  size?: number;
}

export const CircularProgress: React.FC<CircularProgressProps> = ({
  done,
  total,
  failed = 0,
  size = 16,
}) => {
  const hasError = failed > 0;
  const progress = total > 0 ? done / total : 0;
  const strokeWidth = 2;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = circumference * (1 - progress);

  const tooltip = hasError
    ? `${done}/${total} completed, ${failed} failed`
    : `${done}/${total} completed`;

  return (
    <Tooltip delayDuration={200}>
      <Tooltip.Trigger asChild>
        <svg
          width={size}
          height={size}
          viewBox={`0 0 ${size} ${size}`}
          className={styles.root}
          aria-label={tooltip}
        >
          <circle
            className={styles.track}
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            strokeWidth={strokeWidth}
          />
          <circle
            className={classNames(
              styles.progress,
              hasError ? styles.danger : styles.success,
            )}
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            strokeWidth={strokeWidth}
            strokeDasharray={circumference}
            strokeDashoffset={strokeDashoffset}
            strokeLinecap="round"
          />
        </svg>
      </Tooltip.Trigger>
      <Tooltip.Content side="top" align="center">
        <p className={styles.tooltip}>{tooltip}</p>
      </Tooltip.Content>
    </Tooltip>
  );
};
