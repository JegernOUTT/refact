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
  density?: "default" | "compact";
  actions?: React.ReactNode;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  collapsible?: boolean;
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

const collapsedFocusOwners = new WeakMap<
  HTMLElement,
  { tabIndex: string | null; owners: Set<HTMLElement> }
>();

function useCollapsedFocusGuard(
  ref: React.RefObject<HTMLElement>,
  active: boolean,
): void {
  React.useLayoutEffect(() => {
    const root = ref.current;
    if (!root || !active) return;

    const owned = new Set<HTMLElement>();
    const release = (element: HTMLElement) => {
      owned.delete(element);
      const state = collapsedFocusOwners.get(element);
      if (!state) return;
      state.owners.delete(root);
      if (state.owners.size > 0) return;
      if (state.tabIndex === null) element.removeAttribute("tabindex");
      else element.setAttribute("tabindex", state.tabIndex);
      collapsedFocusOwners.delete(element);
    };
    const sync = () => {
      for (const element of owned) {
        if (!root.contains(element)) release(element);
      }
      for (const element of root.querySelectorAll<HTMLElement>(
        focusableSelector,
      )) {
        if (!owned.has(element)) {
          const state = collapsedFocusOwners.get(element) ?? {
            tabIndex: element.getAttribute("tabindex"),
            owners: new Set<HTMLElement>(),
          };
          state.owners.add(root);
          collapsedFocusOwners.set(element, state);
          owned.add(element);
        }
        element.tabIndex = -1;
      }
    };

    root.setAttribute("inert", "");
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(root, { childList: true, subtree: true });
    return () => {
      observer.disconnect();
      root.removeAttribute("inert");
      for (const element of owned) release(element);
    };
  }, [active, ref]);
}

export function ToolCard({
  title,
  icon,
  status = "idle",
  density = "default",
  actions,
  open,
  defaultOpen = true,
  onOpenChange,
  collapsible = true,
  children,
  className,
  ...props
}: ToolCardProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = !collapsible || (isControlled ? open : uncontrolledOpen);
  const bodyId = React.useId();
  const bodyRef = React.useRef<HTMLDivElement>(null);
  const tone = statusTone[status];
  const isActive = status === "running" || status === "streaming";
  const titleContent =
    isActive && (typeof title === "string" || typeof title === "number") ? (
      <span className="rf-text-shimmer">{title}</span>
    ) : (
      title
    );

  useCollapsedFocusGuard(bodyRef, !isOpen);

  const toggleOpen = () => {
    const nextOpen = !isOpen;

    if (!isControlled) {
      setUncontrolledOpen(nextOpen);
    }

    onOpenChange?.(nextOpen);
  };

  const headerContent = (
    <>
      {icon ? (
        <Icon
          className={classNames(
            styles.leadingIcon,
            isActive && "rf-status-pulse",
          )}
          icon={icon}
          tone={tone}
        />
      ) : null}
      <span className={styles.title}>{titleContent}</span>
      <span className={styles.spacer} />
    </>
  );

  return (
    <section
      className={classNames(
        styles.root,
        styles[`status-${status}`],
        density === "compact" && styles.compact,
        className,
      )}
      data-density={density}
      data-open={isOpen}
      data-status={status}
      {...props}
    >
      <div className={styles.header}>
        {collapsible ? (
          <button
            aria-controls={bodyId}
            aria-expanded={isOpen}
            className={classNames(styles.toggle, "rf-pressable")}
            type="button"
            onClick={toggleOpen}
          >
            {headerContent}
            <Icon className={styles.chevron} icon={ChevronDown} tone="faint" />
          </button>
        ) : (
          <div className={classNames(styles.toggle, styles.static)}>
            {headerContent}
          </div>
        )}
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
