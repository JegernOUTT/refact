import React from "react";
import classNames from "classnames";
import { ChevronDown } from "lucide-react";

import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../../shared/useDelayedUnmount";
import styles from "./AnimatedCollapsible.module.css";

export type AnimatedCollapsibleStatus =
  | "idle"
  | "running"
  | "success"
  | "error"
  | "streaming";

export type AnimatedCollapsibleVariant = "card" | "compact";

export interface AnimatedCollapsibleHeaderRenderProps {
  open: boolean;
  status: AnimatedCollapsibleStatus;
}

export interface AnimatedCollapsibleProps
  extends Omit<React.ComponentPropsWithoutRef<"section">, "title"> {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  title?: React.ReactNode;
  header?:
    | React.ReactNode
    | ((props: AnimatedCollapsibleHeaderRenderProps) => React.ReactNode);
  icon?: React.ReactNode;
  actions?: React.ReactNode;
  status?: AnimatedCollapsibleStatus;
  animate?: boolean;
  variant?: AnimatedCollapsibleVariant;
  children?: React.ReactNode;
}

export function AnimatedCollapsible({
  open,
  defaultOpen = false,
  onOpenChange,
  title,
  header,
  icon,
  actions,
  status = "idle",
  animate = true,
  variant = "card",
  children,
  className,
  ...props
}: AnimatedCollapsibleProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : uncontrolledOpen;
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isOpen,
    COLLAPSE_ANIMATION_MS,
    animate,
  );
  const shouldRenderBody = isOpen || shouldRender;
  const bodyId = React.useId();

  const handleToggle = React.useCallback(() => {
    const nextOpen = !isOpen;

    if (!isControlled) {
      setUncontrolledOpen(nextOpen);
    }

    onOpenChange?.(nextOpen);
  }, [isControlled, isOpen, onOpenChange]);

  const headerContent =
    typeof header === "function" ? header({ open: isOpen, status }) : header;

  return (
    <section
      className={classNames(
        styles.root,
        styles[`variant-${variant}`],
        styles[`status-${status}`],
        className,
      )}
      data-animate={animate}
      data-has-icon={icon ? "true" : "false"}
      data-open={isOpen}
      data-status={status}
      {...props}
    >
      <div className={styles.headerRow}>
        <button
          aria-controls={bodyId}
          aria-expanded={isOpen}
          className={classNames(styles.trigger, "rf-pressable")}
          type="button"
          onClick={handleToggle}
        >
          {icon ? <span className={styles.icon}>{icon}</span> : null}
          <span className={styles.headerContent}>{headerContent ?? title}</span>
          <span className={styles.spacer} />
          <ChevronDown aria-hidden className={styles.chevron} />
        </button>
        {actions ? <div className={styles.actions}>{actions}</div> : null}
      </div>
      {shouldRenderBody ? (
        <div
          className={classNames("rf-expand-grid", styles.bodyGrid)}
          data-open={isAnimatingOpen}
          id={bodyId}
        >
          <div className={styles.bodyShell}>
            <div className={styles.body}>{children}</div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
