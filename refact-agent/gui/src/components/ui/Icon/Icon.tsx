import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import styles from "./Icon.module.css";

export interface IconProps {
  icon: LucideIcon;
  size?: "sm" | "md" | "lg";
  tone?:
    | "default"
    | "muted"
    | "faint"
    | "accent"
    | "success"
    | "warning"
    | "danger";
  "aria-label"?: string;
  className?: string;
}

export function Icon({
  icon: LucideIconComponent,
  size = "md",
  tone = "default",
  "aria-label": ariaLabel,
  className,
}: IconProps) {
  return (
    <LucideIconComponent
      aria-hidden={ariaLabel ? undefined : true}
      aria-label={ariaLabel}
      className={classNames(
        styles.icon,
        styles[`size-${size}`],
        styles[`tone-${tone}`],
        className,
      )}
      fill="none"
      focusable="false"
      stroke="currentColor"
      strokeWidth={1.5}
    />
  );
}
