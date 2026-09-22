import React from "react";
import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import { Icon, Text } from "../../components/ui";
import styles from "./BuddySectionHeader.module.css";

interface BuddySectionHeaderProps {
  icon: LucideIcon;
  label: string;
  level?: "h2" | "h3";
  badge?: React.ReactNode;
  actions?: React.ReactNode;
  className?: string;
}

/**
 * Shared pinned header for Buddy home cards: Lucide icon + section heading,
 * optional count badge, and right-aligned actions. Keeps every panel on the
 * page speaking the same visual language.
 */
export const BuddySectionHeader: React.FC<BuddySectionHeaderProps> = ({
  icon,
  label,
  level = "h2",
  badge,
  actions,
  className,
}) => (
  <header className={classNames(styles.header, className)}>
    <span className={styles.labelGroup}>
      <Icon icon={icon} size="sm" tone="muted" />
      <Text as={level} size="3" weight="bold" className={styles.label}>
        {label}
      </Text>
    </span>
    {badge}
    <span className={styles.spacer} aria-hidden />
    {actions}
  </header>
);
