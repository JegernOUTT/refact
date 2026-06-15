import React from "react";
import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import { Icon } from "../Icon";
import styles from "./EmptyState.module.css";

export type EmptyStateVariant = "compact" | "full";

export interface EmptyStateProps
  extends Omit<React.ComponentPropsWithoutRef<"section">, "title"> {
  icon?: LucideIcon;
  title: React.ReactNode;
  description?: React.ReactNode;
  action?: React.ReactNode;
  variant?: EmptyStateVariant;
}

export function EmptyState({
  icon,
  title,
  description,
  action,
  variant = "compact",
  className,
  ...props
}: EmptyStateProps) {
  return (
    <section
      className={classNames(styles.emptyState, styles[variant], className)}
      {...props}
    >
      {icon ? (
        <div className={styles.iconWrap}>
          <Icon
            icon={icon}
            size={variant === "full" ? "lg" : "md"}
            tone="muted"
          />
        </div>
      ) : null}
      <div className={styles.copy}>
        <h3 className={styles.title}>{title}</h3>
        {description ? (
          <p className={styles.description}>{description}</p>
        ) : null}
      </div>
      {action ? <div className={styles.action}>{action}</div> : null}
    </section>
  );
}
