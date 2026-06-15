import classNames from "classnames";
import type React from "react";

import styles from "./SettingsSection.module.css";

export interface SettingsSectionProps
  extends Omit<React.ComponentProps<"section">, "title"> {
  title: React.ReactNode;
  description?: React.ReactNode;
  actions?: React.ReactNode;
  subNav?: React.ReactNode;
  width?: "default" | "wide";
  children: React.ReactNode;
}

export interface SettingsGroupProps
  extends Omit<React.ComponentProps<"section">, "title"> {
  title: React.ReactNode;
  children: React.ReactNode;
}

export function SettingsSection({
  actions,
  children,
  className,
  description,
  subNav,
  title,
  width = "default",
  ...props
}: SettingsSectionProps) {
  return (
    <section
      {...props}
      className={classNames(
        styles.section,
        styles[width],
        "rf-enter-rise",
        className,
      )}
    >
      <div className={styles.header}>
        <div className={styles.copy}>
          <h1 className={styles.title}>{title}</h1>
          {description ? (
            <p className={styles.description}>{description}</p>
          ) : null}
        </div>
        {actions ? <div className={styles.actions}>{actions}</div> : null}
      </div>
      {subNav ? <div className={styles.subNav}>{subNav}</div> : null}
      <div className={styles.body}>{children}</div>
    </section>
  );
}

export function SettingsGroup({
  children,
  className,
  title,
  ...props
}: SettingsGroupProps) {
  return (
    <section
      {...props}
      className={classNames(styles.group, "rf-stagger", className)}
    >
      <h2 className={styles.groupTitle}>{title}</h2>
      <div className={styles.groupRows}>{children}</div>
    </section>
  );
}
