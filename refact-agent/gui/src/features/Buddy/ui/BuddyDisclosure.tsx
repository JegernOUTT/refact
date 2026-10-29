import React, { useCallback, useId, useRef } from "react";
import classNames from "classnames";
import { ChevronRight } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Badge, Icon } from "../../../components/ui";
import { useBuddyPersistentState } from "./useBuddyPersistentState";
import styles from "./BuddyDisclosure.module.css";

export type BuddyDisclosureVariant = "section" | "inline";
export type BuddyHeadingLevel = "h2" | "h3" | "h4";

export interface BuddyDisclosureProps {
  label: React.ReactNode;
  children?: React.ReactNode;
  count?: number;
  summary?: React.ReactNode;
  icon?: LucideIcon;
  trailing?: React.ReactNode;
  headingLevel?: BuddyHeadingLevel;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  persistKey?: string;
  lazy?: boolean;
  variant?: BuddyDisclosureVariant;
  disabled?: boolean;
  id?: string;
  className?: string;
  contentClassName?: string;
  testId?: string;
}

export function BuddyDisclosure({
  label,
  children,
  count,
  summary,
  icon,
  trailing,
  headingLevel,
  open: controlledOpen,
  defaultOpen = false,
  onOpenChange,
  persistKey,
  lazy = false,
  variant = "inline",
  disabled = false,
  id,
  className,
  contentClassName,
  testId,
}: BuddyDisclosureProps) {
  const generatedId = useId();
  const baseId = id ?? `buddy-disclosure-${generatedId.replace(/:/g, "")}`;
  const triggerId = `${baseId}-trigger`;
  const contentId = `${baseId}-content`;

  const [storedOpen, setStoredOpen] = useBuddyPersistentState(
    persistKey,
    defaultOpen,
  );
  const controlled = controlledOpen !== undefined;
  const open = controlled ? controlledOpen : storedOpen;

  const mountedRef = useRef(open);
  if (open) mountedRef.current = true;
  const renderContent = !lazy || mountedRef.current;

  const toggle = useCallback(() => {
    const next = !open;
    if (!controlled) setStoredOpen(next);
    onOpenChange?.(next);
  }, [controlled, onOpenChange, open, setStoredOpen]);

  const trigger = (
    <button
      type="button"
      id={triggerId}
      className={classNames(styles.trigger, "rf-focus-ring")}
      aria-expanded={open}
      aria-controls={contentId}
      disabled={disabled}
      onClick={toggle}
      data-testid={testId ? `${testId}-trigger` : undefined}
    >
      <Icon icon={ChevronRight} size="sm" className={styles.chevron} />
      {icon ? <Icon icon={icon} size="md" tone="muted" /> : null}
      <span className={styles.label}>{label}</span>
      {count !== undefined ? (
        <Badge size="xs" tone="default" className={styles.count}>
          {count}
        </Badge>
      ) : null}
      {summary ? <span className={styles.summary}>{summary}</span> : null}
    </button>
  );

  const Heading = headingLevel;

  return (
    <div
      className={classNames(styles.disclosure, styles[variant], className)}
      data-state={open ? "open" : "closed"}
      data-testid={testId}
    >
      <div className={styles.header}>
        {Heading ? (
          <Heading className={styles.heading}>{trigger}</Heading>
        ) : (
          trigger
        )}
        {trailing ? <div className={styles.trailing}>{trailing}</div> : null}
      </div>
      <div
        id={contentId}
        role="region"
        aria-labelledby={triggerId}
        hidden={!open}
        className={classNames(
          styles.content,
          open && "rf-enter",
          contentClassName,
        )}
      >
        {renderContent ? children : null}
      </div>
    </div>
  );
}
