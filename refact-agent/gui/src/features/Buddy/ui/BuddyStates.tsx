import React from "react";
import classNames from "classnames";
import { TriangleAlert } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Button, EmptyState, Icon, Skeleton } from "../../../components/ui";
import { BuddyDisclosure } from "./BuddyDisclosure";
import { BUDDY_TERMS, BUDDY_VERBS, buddyLoadError } from "./buddyGlossary";
import styles from "./BuddyStates.module.css";

export interface BuddyEmptyStateProps {
  icon?: LucideIcon;
  title: React.ReactNode;
  hint?: React.ReactNode;
  action?: React.ReactNode;
  compact?: boolean;
  className?: string;
  testId?: string;
}

export function BuddyEmptyState({
  icon,
  title,
  hint,
  action,
  compact = false,
  className,
  testId,
}: BuddyEmptyStateProps) {
  if (compact) {
    return (
      <div
        className={classNames(styles.emptyCompact, className)}
        data-testid={testId}
      >
        {icon ? <Icon icon={icon} size="md" tone="muted" /> : null}
        <span className={styles.emptyCopy}>
          <span className={styles.emptyTitle}>{title}</span>
          {hint ? <span className={styles.hint}>{hint}</span> : null}
        </span>
        {action ? <span className={styles.inlineAction}>{action}</span> : null}
      </div>
    );
  }
  return (
    <EmptyState
      icon={icon}
      title={title}
      description={hint}
      action={action}
      variant="compact"
      className={classNames(styles.empty, className)}
      data-testid={testId}
    />
  );
}

function rawErrorMessage(error: unknown): string | null {
  if (error === null || error === undefined) return null;
  if (typeof error === "string") return error.trim() || null;
  if (error instanceof Error) return error.message || null;
  if (typeof error === "object") {
    const record = error as Record<string, unknown>;
    const nested = record.data ?? record.error ?? record.message;
    if (typeof nested === "string") return nested;
    if (nested && typeof nested === "object") {
      const detail = (nested as Record<string, unknown>).detail;
      if (typeof detail === "string") return detail;
    }
    try {
      return JSON.stringify(error);
    } catch {
      return null;
    }
  }
  return String(error);
}

export interface BuddyErrorCalloutProps {
  thing: string;
  title?: React.ReactNode;
  error?: unknown;
  details?: string;
  onRetry?: () => void;
  retrying?: boolean;
  retryLabel?: string;
  compact?: boolean;
  className?: string;
  testId?: string;
}

export function BuddyErrorCallout({
  thing,
  title,
  error,
  details,
  onRetry,
  retrying = false,
  retryLabel = BUDDY_VERBS.retry,
  compact = false,
  className,
  testId,
}: BuddyErrorCalloutProps) {
  const technical = details ?? rawErrorMessage(error);
  return (
    <div
      role="alert"
      className={classNames(
        styles.error,
        compact && styles.errorCompact,
        className,
      )}
      data-testid={testId}
    >
      <div className={styles.errorLine}>
        <Icon icon={TriangleAlert} size="md" tone="danger" />
        <span className={styles.errorTitle}>
          {title ?? buddyLoadError(thing)}
        </span>
        {onRetry ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={onRetry}
            loading={retrying}
            disabled={retrying}
            className={styles.retry}
          >
            {retryLabel}
          </Button>
        ) : null}
      </div>
      {technical ? (
        <BuddyDisclosure
          label={BUDDY_TERMS.technicalDetails}
          className={styles.details}
        >
          <pre className={styles.raw}>{technical}</pre>
        </BuddyDisclosure>
      ) : null}
    </div>
  );
}

export interface BuddyLoadingProps {
  variant?: "rows" | "cards";
  count?: number;
  compact?: boolean;
  label?: string;
  className?: string;
  testId?: string;
}

export function BuddyLoading({
  variant = "rows",
  count = 3,
  compact = false,
  label = "Loading",
  className,
  testId,
}: BuddyLoadingProps) {
  return (
    <div
      role="status"
      aria-label={label}
      aria-live="polite"
      aria-busy="true"
      className={classNames(
        styles.loading,
        styles[variant],
        compact && styles.loadingCompact,
        className,
      )}
      data-testid={testId}
    >
      <span className={styles.srOnly}>{label}</span>
      {Array.from({ length: count }).map((_, index) =>
        variant === "cards" ? (
          <div key={index} className={styles.cardShell} aria-hidden="true">
            <Skeleton className={styles.cardTitle} radius="chip" />
            <Skeleton className={styles.lineLong} radius="chip" />
            <Skeleton className={styles.lineShort} radius="chip" />
          </div>
        ) : (
          <div key={index} className={styles.rowShell} aria-hidden="true">
            <Skeleton className={styles.rowTick} radius="pill" />
            <div className={styles.rowLines}>
              <Skeleton className={styles.lineLong} radius="chip" />
              {compact ? null : (
                <Skeleton className={styles.lineShort} radius="chip" />
              )}
            </div>
          </div>
        ),
      )}
    </div>
  );
}
