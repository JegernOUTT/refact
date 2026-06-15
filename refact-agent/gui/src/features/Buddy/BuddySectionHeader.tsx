import React from "react";
import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import { Icon } from "../../components/ui";
import styles from "./BuddySectionHeader.module.css";

interface BuddySectionHeaderProps {
  icon: LucideIcon;
  label: string;
  badge?: React.ReactNode;
  actions?: React.ReactNode;
  className?: string;
}

/**
 * Shared pinned header for Buddy home cards: Lucide icon + uppercase label,
 * optional count badge, and right-aligned actions. Keeps every panel on the
 * page speaking the same visual language.
 */
export const BuddySectionHeader: React.FC<BuddySectionHeaderProps> = ({
  icon,
  label,
  badge,
  actions,
  className,
}) => (
  <header className={classNames(styles.header, className)}>
    <span className={styles.labelGroup}>
      <Icon icon={icon} size="sm" tone="muted" />
      <span className={styles.label}>{label}</span>
    </span>
    {badge}
    <span className={styles.spacer} aria-hidden />
    {actions}
  </header>
);
