import React from "react";
import type { LucideIcon } from "lucide-react";
import { Activity, ArrowUpRight } from "lucide-react";
import { Card, Icon, StatusDot } from "../../../components/ui";
import type { StatusDotProps } from "../../../components/ui";
import styles from "./StatCard.module.css";

export type StatCardProps = {
  title: string;
  value: string;
  subtitle?: string;
  icon?: LucideIcon;
  tone?: "accent" | "success" | "warning" | "danger" | "muted";
};

const dotStatus: Record<
  NonNullable<StatCardProps["tone"]>,
  StatusDotProps["status"]
> = {
  accent: "running",
  success: "success",
  warning: "warning",
  danger: "error",
  muted: "idle",
};

export const StatCard: React.FC<StatCardProps> = ({
  title,
  value,
  subtitle,
  icon = Activity,
  tone = "accent",
}) => (
  <Card animated="rise" className={styles.card}>
    <div className={styles.header}>
      <span className={styles.iconShell}>
        <Icon icon={icon} size="md" tone={tone === "muted" ? "muted" : tone} />
      </span>
      <StatusDot status={dotStatus[tone]} />
    </div>
    <p className={styles.title}>{title}</p>
    <div className={styles.valueRow}>
      <strong className={styles.value}>{value}</strong>
      <Icon
        className={styles.trendIcon}
        icon={ArrowUpRight}
        size="sm"
        tone="faint"
      />
    </div>
    {subtitle && <p className={styles.subtitle}>{subtitle}</p>}
  </Card>
);
