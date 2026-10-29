import React from "react";
import classNames from "classnames";
import { ChevronRight } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Icon } from "../../../components/ui";
import type { BuddyStatusTone } from "./buddyStatus";
import styles from "./BuddyRow.module.css";

export const BuddyInteractiveRowContext = React.createContext(false);

export type BuddyRowDensity = "regular" | "compact";
export type BuddyRowSelection = "current" | "selected";

export type BuddyRowLeading =
  | { tick: BuddyStatusTone }
  | { icon: LucideIcon; tone?: "muted" | "default" | BuddyStatusTone };

type MainAttributes = Omit<
  React.HTMLAttributes<HTMLElement>,
  "title" | "children" | "onClick"
> &
  Record<`data-${string}`, string | number | boolean | undefined>;

export interface BuddyRowProps extends MainAttributes {
  title: React.ReactNode;
  fullTitle?: string;
  titleLines?: 1 | 2;
  description?: React.ReactNode;
  leading?: BuddyRowLeading;
  status?: React.ReactNode;
  meta?: React.ReactNode[];
  actions?: React.ReactNode;
  chevron?: boolean;
  onClick?: React.MouseEventHandler<HTMLElement>;
  href?: string;
  target?: string;
  rel?: string;
  selected?: boolean;
  selection?: BuddyRowSelection;
  unread?: boolean;
  disabled?: boolean;
  density?: BuddyRowDensity;
  className?: string;
  testId?: string;
}

const ICON_TONE: Record<
  NonNullable<Extract<BuddyRowLeading, { icon: LucideIcon }>["tone"]>,
  React.ComponentProps<typeof Icon>["tone"]
> = {
  muted: "muted",
  default: "muted",
  success: "success",
  warning: "warning",
  danger: "danger",
};

function joinMeta(meta: React.ReactNode[] | undefined): React.ReactNode {
  const parts = (meta ?? []).filter(
    (part) =>
      part !== null && part !== undefined && part !== false && part !== "",
  );
  if (parts.length === 0) return null;
  return parts.map((part, index) => (
    <React.Fragment key={index}>
      {index > 0 ? (
        <span className={styles.separator} aria-hidden="true">
          {" · "}
        </span>
      ) : null}
      <span className={styles.metaPart}>{part}</span>
    </React.Fragment>
  ));
}

export const BuddyRow = React.forwardRef<HTMLElement, BuddyRowProps>(
  function BuddyRow(
    {
      title,
      fullTitle,
      titleLines = 1,
      description,
      leading,
      status,
      meta,
      actions,
      chevron,
      onClick,
      href,
      target,
      rel,
      selected = false,
      selection = "current",
      unread = false,
      disabled = false,
      density = "regular",
      className,
      testId,
      ...mainProps
    },
    ref,
  ) {
    const kind = href ? "link" : onClick ? "button" : "static";
    const interactive = kind !== "static";
    const showChevron = chevron ?? (interactive && !actions);
    const titleText =
      fullTitle ?? (typeof title === "string" ? title : undefined);
    const metaContent = joinMeta(meta);

    const content = (
      <>
        {leading ? (
          "tick" in leading ? (
            <span
              className={styles.tick}
              data-tone={leading.tick}
              aria-hidden="true"
            />
          ) : (
            <span className={styles.leadingIcon}>
              <Icon
                icon={leading.icon}
                size="md"
                tone={ICON_TONE[leading.tone ?? "muted"]}
              />
            </span>
          )
        ) : null}
        <span className={styles.body}>
          <span className={styles.titleLine}>
            {unread ? (
              <span className={styles.unreadDot} aria-hidden="true" />
            ) : null}
            <span
              className={classNames(
                styles.title,
                titleLines === 2 ? styles.clamp2 : styles.clamp1,
              )}
              title={titleText}
            >
              {title}
            </span>
            {unread ? <span className={styles.srOnly}> (unread)</span> : null}
            {status ? <span className={styles.status}>{status}</span> : null}
          </span>
          {description ? (
            <span className={styles.description}>{description}</span>
          ) : null}
          {metaContent ? (
            <span className={styles.meta}>{metaContent}</span>
          ) : null}
        </span>
        {showChevron ? (
          <Icon icon={ChevronRight} size="sm" className={styles.chevron} />
        ) : null}
      </>
    );

    const mainClass = classNames(
      styles.main,
      interactive && styles.interactive,
      interactive && "rf-focus-ring-inset",
    );
    const selectionAttrs = interactive
      ? selection === "current"
        ? { "aria-current": selected ? ("true" as const) : undefined }
        : { "aria-selected": selected }
      : {};

    let main: React.ReactNode;
    if (kind === "link") {
      main = (
        <a
          {...mainProps}
          {...selectionAttrs}
          ref={ref as React.Ref<HTMLAnchorElement>}
          className={mainClass}
          href={disabled ? undefined : href}
          target={target}
          rel={rel}
          aria-disabled={disabled || undefined}
          onClick={disabled ? undefined : onClick}
        >
          {content}
        </a>
      );
    } else if (kind === "button") {
      main = (
        <button
          {...mainProps}
          {...selectionAttrs}
          ref={ref as React.Ref<HTMLButtonElement>}
          type="button"
          className={mainClass}
          disabled={disabled}
          onClick={onClick}
        >
          {content}
        </button>
      );
    } else {
      main = (
        <div
          {...mainProps}
          ref={ref as React.Ref<HTMLDivElement>}
          className={mainClass}
          aria-disabled={disabled || undefined}
        >
          {content}
        </div>
      );
    }

    return (
      <div
        className={classNames(styles.row, styles[density], className)}
        data-kind={kind}
        data-selected={selected || undefined}
        data-unread={unread || undefined}
        data-disabled={disabled || undefined}
        data-testid={testId}
      >
        <BuddyInteractiveRowContext.Provider value={interactive}>
          {main}
        </BuddyInteractiveRowContext.Provider>
        {actions ? <div className={styles.actions}>{actions}</div> : null}
      </div>
    );
  },
);
