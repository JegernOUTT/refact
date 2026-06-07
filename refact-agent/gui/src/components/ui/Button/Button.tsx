import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import React from "react";
import { LoaderCircle } from "lucide-react";
import { Icon } from "../Icon";
import styles from "./Button.module.css";

export type ButtonVariant = "ghost" | "soft" | "primary" | "danger" | "plain";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends React.ComponentPropsWithoutRef<"button"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  leftIcon?: LucideIcon;
  rightIcon?: LucideIcon;
}

export interface IconButtonProps
  extends Omit<React.ComponentPropsWithoutRef<"button">, "aria-label" | "children"> {
  "aria-label": string;
  icon: LucideIcon;
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}

export interface ButtonGroupProps extends React.ComponentProps<"div"> {
  children?: React.ReactNode;
}

function getIconTone(variant: ButtonVariant): React.ComponentProps<typeof Icon>["tone"] {
  if (variant === "primary") {
    return "accent";
  }

  if (variant === "danger") {
    return "danger";
  }

  return "default";
}

function getIconSize(size: ButtonSize): React.ComponentProps<typeof Icon>["size"] {
  if (size === "sm") {
    return "sm";
  }

  if (size === "lg") {
    return "lg";
  }

  return "md";
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      children,
      className,
      disabled,
      leftIcon,
      loading = false,
      rightIcon,
      size = "md",
      type = "button",
      variant = "ghost",
      ...props
    },
    ref,
  ) => {
    const isDisabled = disabled === true || loading;
    const iconTone = getIconTone(variant);
    const iconSize = getIconSize(size);

    return (
      <button
        {...props}
        ref={ref}
        aria-busy={loading ? true : props["aria-busy"]}
        className={classNames(
          styles.button,
          styles[`variant-${variant}`],
          styles[`size-${size}`],
          "rf-pressable",
          className,
        )}
        disabled={isDisabled}
        type={type}
      >
        {loading ? (
          <span aria-hidden="true" className={styles.spinner}>
            <Icon icon={LoaderCircle} size={iconSize} tone={iconTone} />
          </span>
        ) : leftIcon ? (
          <Icon icon={leftIcon} size={iconSize} tone={iconTone} />
        ) : null}
        <span className={styles.label}>{children}</span>
        {rightIcon && !loading ? (
          <Icon icon={rightIcon} size={iconSize} tone={iconTone} />
        ) : null}
      </button>
    );
  },
);
Button.displayName = "Button";

export function ButtonGroup({ children, className, ...props }: ButtonGroupProps) {
  return (
    <div {...props} className={classNames(styles.group, className)}>
      {children}
    </div>
  );
}

export const IconButton = React.forwardRef<HTMLButtonElement, IconButtonProps>(
  (
    {
      "aria-label": ariaLabel,
      className,
      disabled,
      icon,
      loading = false,
      size = "md",
      type = "button",
      variant = "ghost",
      ...props
    },
    ref,
  ) => {
    const isDisabled = disabled === true || loading;
    const iconTone = getIconTone(variant);
    const iconSize = getIconSize(size);

    return (
      <button
        {...props}
        ref={ref}
        aria-busy={loading ? true : props["aria-busy"]}
        aria-label={ariaLabel}
        className={classNames(
          styles.iconButton,
          styles[`variant-${variant}`],
          styles[`size-${size}`],
          "rf-pressable",
          className,
        )}
        disabled={isDisabled}
        type={type}
      >
        <span aria-hidden="true" className={loading ? styles.spinner : undefined}>
          <Icon icon={loading ? LoaderCircle : icon} size={iconSize} tone={iconTone} />
        </span>
      </button>
    );
  },
);
IconButton.displayName = "IconButton";
