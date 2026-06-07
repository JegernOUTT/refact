import classNames from "classnames";
import type { LucideIcon } from "lucide-react";
import React from "react";
import { LoaderCircle } from "lucide-react";
import { Icon } from "../Icon";
import styles from "./Button.module.css";

export type ButtonVariant =
  | "ghost"
  | "soft"
  | "primary"
  | "danger"
  | "plain"
  | "solid"
  | "outline";
export type ButtonSize = "sm" | "md" | "lg" | "1" | "2" | "3";

export interface ButtonProps extends React.ComponentPropsWithoutRef<"button"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  asChild?: boolean;
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

function normalizeVariant(variant: ButtonVariant): Exclude<ButtonVariant, "solid" | "outline"> {
  if (variant === "solid") return "primary";
  if (variant === "outline") return "soft";
  return variant;
}

function normalizeSize(size: ButtonSize): Exclude<ButtonSize, "1" | "2" | "3"> {
  if (size === "1") return "sm";
  if (size === "3") return "lg";
  if (size === "2") return "md";
  return size;
}

function getIconTone(variant: ButtonVariant): React.ComponentProps<typeof Icon>["tone"] {
  variant = normalizeVariant(variant);
  if (variant === "primary") {
    return "accent";
  }

  if (variant === "danger") {
    return "danger";
  }

  return "default";
}

function getIconSize(size: ButtonSize): React.ComponentProps<typeof Icon>["size"] {
  size = normalizeSize(size);
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
      asChild: _asChild,
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
    const normalizedVariant = normalizeVariant(variant);
    const normalizedSize = normalizeSize(size);
    const iconTone = getIconTone(normalizedVariant);
    const iconSize = getIconSize(normalizedSize);

    return (
      <button
        {...props}
        ref={ref}
        aria-busy={loading ? true : props["aria-busy"]}
        className={classNames(
          styles.button,
          styles[`variant-`],
          styles[`size-`],
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
    const normalizedVariant = normalizeVariant(variant);
    const normalizedSize = normalizeSize(size);
    const iconTone = getIconTone(normalizedVariant);
    const iconSize = getIconSize(normalizedSize);

    return (
      <button
        {...props}
        ref={ref}
        aria-busy={loading ? true : props["aria-busy"]}
        aria-label={ariaLabel}
        className={classNames(
          styles.iconButton,
          styles[`variant-`],
          styles[`size-`],
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
