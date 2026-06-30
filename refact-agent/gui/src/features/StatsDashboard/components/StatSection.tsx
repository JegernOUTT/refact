import React from "react";
import type { LucideIcon } from "lucide-react";
import { Icon } from "../../../components/ui";
import styles from "./StatSection.module.css";

export type StatSectionProps = {
  title: string;
  icon?: LucideIcon;
  /** Optional right-aligned content in the section header. */
  actions?: React.ReactNode;
  /** When set, children flow in a denser min-column grid. */
  dense?: boolean;
  children: React.ReactNode;
};

export const StatSection: React.FC<StatSectionProps> = ({
  title,
  icon,
  actions,
  dense = false,
  children,
}) => (
  <section className={styles.section}>
    <div className={styles.header}>
      <h3 className={styles.title}>
        {icon && <Icon icon={icon} size="sm" tone="accent" />}
        {title}
      </h3>
      {actions && <div className={styles.actions}>{actions}</div>}
    </div>
    <div className={dense ? styles.gridDense : styles.grid}>{children}</div>
  </section>
);
