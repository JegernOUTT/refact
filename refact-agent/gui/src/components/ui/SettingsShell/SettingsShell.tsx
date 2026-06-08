import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import type React from "react";

import { Icon } from "../Icon";
import { Select } from "../Select";
import styles from "./SettingsShell.module.css";

export interface SettingsShellSection {
  id: string;
  label: string;
  icon?: LucideIcon;
}

export interface SettingsShellProps
  extends Omit<React.ComponentProps<"div">, "title"> {
  sections: SettingsShellSection[];
  active: string;
  onSectionChange: (sectionId: string) => void;
  children?: React.ReactNode;
  navLabel?: string;
  title?: React.ReactNode;
  description?: React.ReactNode;
}

export function SettingsShell({
  active,
  children,
  className,
  description,
  navLabel = "Settings sections",
  onSectionChange,
  sections,
  title,
  ...props
}: SettingsShellProps) {
  const activeSection =
    sections.find((section) => section.id === active) ?? sections[0];

  return (
    <div {...props} className={classNames(styles.shell, className)}>
      <div className={styles.mobileNav}>
        {title ?? description ? (
          <div className={styles.mobileHeader}>
            {title ? <h2 className={styles.title}>{title}</h2> : null}
            {description ? (
              <p className={styles.description}>{description}</p>
            ) : null}
          </div>
        ) : null}
        <Select value={activeSection.id} onValueChange={onSectionChange}>
          <Select.Trigger
            aria-label={navLabel}
            className={styles.sectionSelect}
          />
          <Select.Content maxHeight="260px" maxWidth="360px">
            {sections.map((section) => (
              <Select.Item key={section.id} value={section.id}>
                {section.label}
              </Select.Item>
            ))}
          </Select.Content>
        </Select>
      </div>
      <aside aria-label={navLabel} className={styles.sidebar}>
        {title ?? description ? (
          <div className={styles.header}>
            {title ? <h2 className={styles.title}>{title}</h2> : null}
            {description ? (
              <p className={styles.description}>{description}</p>
            ) : null}
          </div>
        ) : null}
        <nav className={styles.nav}>
          {sections.map((section) => (
            <button
              aria-current={section.id === active ? "page" : undefined}
              className={styles.navItem}
              key={section.id}
              type="button"
              onClick={() => onSectionChange(section.id)}
            >
              {section.icon ? (
                <Icon icon={section.icon} size="sm" tone="muted" />
              ) : null}
              <span>{section.label}</span>
            </button>
          ))}
        </nav>
      </aside>
      <section className={styles.content}>{children}</section>
    </div>
  );
}
