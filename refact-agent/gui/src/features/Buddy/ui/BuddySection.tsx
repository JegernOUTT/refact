import React, { useId } from "react";
import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import { Badge, Icon } from "../../../components/ui";
import { BuddyDisclosure, type BuddyHeadingLevel } from "./BuddyDisclosure";
import { BuddySurface } from "./BuddySurface";
import styles from "./BuddySection.module.css";

export interface BuddySectionCollapsible {
  persistKey?: string;
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  lazy?: boolean;
}

export interface BuddySectionProps {
  title: React.ReactNode;
  icon?: LucideIcon;
  headingLevel?: BuddyHeadingLevel;
  count?: number;
  description?: React.ReactNode;
  action?: React.ReactNode;
  footer?: React.ReactNode;
  collapsible?: boolean | BuddySectionCollapsible;
  children?: React.ReactNode;
  id?: string;
  className?: string;
  bodyClassName?: string;
  testId?: string;
  "aria-label"?: string;
}

export function BuddySection({
  title,
  icon,
  headingLevel = "h2",
  count,
  description,
  action,
  footer,
  collapsible,
  children,
  id,
  className,
  bodyClassName,
  testId,
  "aria-label": ariaLabel,
}: BuddySectionProps) {
  const generatedId = useId();
  const baseId = id ?? `buddy-section-${generatedId.replace(/:/g, "")}`;
  const headingId = `${baseId}-title`;
  const descriptionId = description ? `${baseId}-description` : undefined;

  const body = (
    <>
      {description ? (
        <p id={descriptionId} className={styles.description}>
          {description}
        </p>
      ) : null}
      {children !== undefined && children !== null ? (
        <div className={classNames(styles.body, bodyClassName)}>{children}</div>
      ) : null}
      {footer ? <div className={styles.footer}>{footer}</div> : null}
    </>
  );

  if (collapsible) {
    const options = collapsible === true ? {} : collapsible;
    return (
      <BuddySurface
        as="section"
        level="section"
        className={classNames(styles.section, className)}
        aria-label={ariaLabel}
        testId={testId}
      >
        <BuddyDisclosure
          id={baseId}
          variant="section"
          label={title}
          icon={icon}
          count={count}
          headingLevel={headingLevel}
          trailing={action}
          persistKey={options.persistKey}
          defaultOpen={options.defaultOpen}
          open={options.open}
          onOpenChange={options.onOpenChange}
          lazy={options.lazy}
          contentClassName={styles.collapsibleBody}
        >
          {body}
        </BuddyDisclosure>
      </BuddySurface>
    );
  }

  const Heading = headingLevel;

  return (
    <BuddySurface
      as="section"
      level="section"
      className={classNames(styles.section, className)}
      aria-labelledby={ariaLabel ? undefined : headingId}
      aria-label={ariaLabel}
      aria-describedby={descriptionId}
      testId={testId}
    >
      <header className={styles.header}>
        <div className={styles.titleGroup}>
          {icon ? (
            <Icon icon={icon} size="md" tone="muted" className={styles.icon} />
          ) : null}
          <Heading id={headingId} className={styles.title}>
            {title}
          </Heading>
          {count !== undefined ? (
            <Badge size="xs" tone="default" className={styles.count}>
              {count}
            </Badge>
          ) : null}
        </div>
        {action ? <div className={styles.action}>{action}</div> : null}
      </header>
      {body}
    </BuddySurface>
  );
}
