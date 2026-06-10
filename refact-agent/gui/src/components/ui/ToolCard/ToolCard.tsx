import React from "react";
import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import { ChevronDown } from "lucide-react";

import { Icon } from "../Icon";
import styles from "./ToolCard.module.css";

export type ToolCardStatus =
  | "idle"
  | "running"
  | "success"
  | "error"
  | "streaming";

export interface ToolCardProps
  extends Omit<React.ComponentPropsWithoutRef<"section">, "title"> {
  title: React.ReactNode;
  icon?: LucideIcon;
  status?: ToolCardStatus;
  actions?: React.ReactNode;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children?: React.ReactNode;
}

const statusTone: Record<
  ToolCardStatus,
  "muted" | "accent" | "success" | "danger"
> = {
  idle: "muted",
  running: "accent",
  success: "success",
  error: "danger",
  streaming: "accent",
};

const focusableSelector = [
  "a[href]",
  "area[href]",
  "button",
  "input",
  "select",
  "textarea",
  "summary",
  "iframe",
  "object",
  "embed",
  "audio[controls]",
  "video[controls]",
  "[contenteditable='true']",
  "[tabindex]",
].join(",");

function useCollapsedFocusGuard(
  ref: React.RefObject<HTMLElement>,
  active: boolean,
): void {
  React.useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;

    const elements = Array.from(
      root.querySelectorAll<HTMLElement>(focusableSelector),
    );

    if (active) {
      root.setAttribute("inert", "");
      for (const element of elements) {
        if (element.dataset.rfPreviousTabIndex !== undefined) continue;
        element.dataset.rfPreviousTabIndex = element.getAttribute("tabindex") ?? "";
        element.tabIndex = -1;
      }
      return;
    }

    root.removeAttribute("inert");
    for (const element of elements) {
      const previousTabIndex = element.dataset.rfPreviousTabIndex;
      if (previousTabIndex === undefined) continue;
      if (previousTabIndex) {
        element.setAttribute("tabindex", previousTabIndex);
      } else {
        element.removeAttribute("tabindex");
      }
      delete element.dataset.rfPreviousTabIndex;
    }
  });
}

export function ToolCard({
  title,
  icon,
  status = "idle",
  actions,
  open,
  defaultOpen = true,
  onOpenChange,
  children,
  className,
  ...props
}: ToolCardProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : uncontrolledOpen;
  const bodyId = React.useId();
  const bodyRef = React.useRef<HTMLDivElement>(null);
  const tone = statusTone[status];

  useCollapsedFocusGuard(bodyRef, !isOpen);

  const toggleOpen = () => {
    const nextOpen = !isOpen;

    if (!isControlled) {
      setUncontrolledOpen(nextOpen);
    }

    onOpenChange?.(nextOpen);
  };

  return (
    <section
      className={classNames(styles.root, styles[`status-${status}`], className)}
      data-open={isOpen}
      data-status={status}
      {...props}
    >
      <div className={styles.header}>
        <button
          aria-controls={bodyId}
          aria-expanded={isOpen}
          className={classNames(
            styles.toggle,
            "rf-pressable",
            (status === "running" || status === "streaming") &&
              "rf-active-pulse",
          )}
          type="button"
          onClick={toggleOpen}
        >
          {icon ? (
            <Icon className={styles.leadingIcon} icon={icon} tone={tone} />
          ) : null}
          <span className={styles.title}>{title}</span>
          <span className={styles.spacer} />
          <Icon className={styles.chevron} icon={ChevronDown} tone="faint" />
        </button>
        {actions ? <div className={styles.actions}>{actions}</div> : null}
      </div>
      <div
        ref={bodyRef}
        className={classNames("rf-expand-grid", styles.bodyGrid)}
        data-open={isOpen}
        id={bodyId}
      >
        <div className={styles.bodyShell}>
          <div className={styles.body}>{children}</div>
        </div>
      </div>
    </section>
  );
}
