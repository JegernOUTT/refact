import { useId } from "react";
import type React from "react";

import styles from "./ProviderQuota.module.css";

export type ProviderQuotaTone =
  | "default"
  | "accent"
  | "success"
  | "warning"
  | "danger"
  | "muted";

export interface ProviderQuotaProps
  extends Omit<React.ComponentPropsWithoutRef<"div">, "children"> {
  label: React.ReactNode;
  value: React.ReactNode;
  meta?: React.ReactNode;
  badge?: React.ReactNode;
  usedPercent?: number;
  tone?: ProviderQuotaTone;
}

const toneClass: Record<ProviderQuotaTone, string> = {
  default: styles.toneDefault,
  accent: styles.toneAccent,
  success: styles.toneSuccess,
  warning: styles.toneWarning,
  danger: styles.toneDanger,
  muted: styles.toneMuted,
};

export function ProviderQuota({
  badge,
  className,
  label,
  meta,
  tone = "accent",
  usedPercent,
  value,
  ...props
}: ProviderQuotaProps) {
  const labelId = useId();
  const visualPercent =
    usedPercent === undefined
      ? undefined
      : Number.isFinite(usedPercent)
        ? Math.min(100, Math.max(0, usedPercent))
        : 0;

  return (
    <div
      {...props}
      className={[styles.root, toneClass[tone], className]
        .filter(Boolean)
        .join(" ")}
      data-tone={tone}
    >
      <div className={styles.header}>
        <span className={styles.labelGroup}>
          <span className={styles.label} id={labelId}>
            {label}
          </span>
          {badge != null ? <span className={styles.badge}>{badge}</span> : null}
        </span>
        <span className={styles.value}>{value}</span>
      </div>
      {meta != null ? <div className={styles.meta}>{meta}</div> : null}
      {visualPercent !== undefined ? (
        <progress
          aria-labelledby={labelId}
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={visualPercent}
          className={styles.progress}
          max={100}
          value={visualPercent}
        />
      ) : null}
    </div>
  );
}
