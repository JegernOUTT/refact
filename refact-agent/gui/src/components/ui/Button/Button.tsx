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
  extends Omit<
    React.ComponentPropsWithoutRef<"button">,
    "aria-label" | "children"
  > {
  "aria-label": string;
  icon: LucideIcon;
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}

export interface ButtonGroupProps extends React.ComponentProps<"div"> {
  children?: React.ReactNode;
}

type ButtonChildProps = React.HTMLAttributes<HTMLElement> & {
  disabled?: boolean;
  type?: string;
};

function normalizeVariant(
  variant: ButtonVariant,
): Exclude<ButtonVariant, "solid" | "outline"> {
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

function getIconTone(
  variant: ButtonVariant,
): React.ComponentProps<typeof Icon>["tone"] {
  variant = normalizeVariant(variant);
  if (variant === "primary") {
    return "accent";
  }

  if (variant === "danger") {
    return "danger";
  }

  return "default";
}

function getIconSize(
  size: ButtonSize,
): React.ComponentProps<typeof Icon>["size"] {
  size = normalizeSize(size);
  if (size === "sm") {
    return "sm";
  }

  if (size === "lg") {
    return "lg";
  }

  return "md";
}

function setRef<T>(ref: React.Ref<T> | undefined, value: T | null) {
  if (!ref) return;

  if (typeof ref === "function") {
    ref(value);
    return;
  }

  (ref as React.MutableRefObject<T | null>).current = value;
}

function composeRefs<T>(...refs: (React.Ref<T> | undefined)[]) {
  return (value: T | null) => {
    for (const ref of refs) {
      setRef(ref, value);
    }
  };
}

function canReceiveDisabledAttribute(type: unknown) {
  return (
    typeof type === "string" &&
    (type === "button" ||
      type === "fieldset" ||
      type === "input" ||
      type === "optgroup" ||
      type === "option" ||
      type === "select" ||
      type === "textarea")
  );
}

function hasRenderableContent(children: React.ReactNode) {
  return React.Children.toArray(children).some(
    (child) => typeof child !== "string" || child.trim().length > 0,
  );
}

function isSingleIconContent(children: React.ReactNode) {
  const renderableChildren = React.Children.toArray(children);

  if (renderableChildren.length !== 1) {
    return false;
  }

  const child = renderableChildren[0];

  if (!React.isValidElement<{ children?: React.ReactNode }>(child)) {
    return false;
  }

  return !hasRenderableContent(child.props.children);
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      children,
      asChild = false,
      className,
      disabled,
      leftIcon,
      loading = false,
      onClick,
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
    const getButtonClassName = (iconOnly: boolean) =>
      classNames(
        styles.button,
        styles[`variant-${normalizedVariant}`],
        styles[`size-${normalizedSize}`],
        iconOnly && styles.iconOnly,
        "rf-pressable",
        className,
      );
    const isIconOnly = (label: React.ReactNode) => {
      const showLabel = hasRenderableContent(label);

      if (!showLabel) {
        return leftIcon !== undefined || rightIcon !== undefined || loading;
      }

      return !leftIcon && !rightIcon && !loading && isSingleIconContent(label);
    };
    const renderContent = (label: React.ReactNode) => {
      const showLabel = hasRenderableContent(label);
      const labelIsIconOnly = showLabel && isSingleIconContent(label);

      return (
        <>
          {loading ? (
            <span
              aria-hidden="true"
              className={classNames(styles.icon, styles.spinner)}
            >
              <Icon icon={LoaderCircle} size={iconSize} tone={iconTone} />
            </span>
          ) : leftIcon ? (
            <span aria-hidden="true" className={styles.icon}>
              <Icon icon={leftIcon} size={iconSize} tone={iconTone} />
            </span>
          ) : null}
          {showLabel ? (
            labelIsIconOnly ? (
              <span aria-hidden="true" className={styles.icon}>
                {label}
              </span>
            ) : (
              <span className={styles.label}>{label}</span>
            )
          ) : null}
          {rightIcon && !loading ? (
            <span aria-hidden="true" className={styles.icon}>
              <Icon icon={rightIcon} size={iconSize} tone={iconTone} />
            </span>
          ) : null}
        </>
      );
    };

    if (asChild) {
      const child = React.Children.only(children);

      if (!React.isValidElement<ButtonChildProps>(child)) {
        throw new Error(
          "Button with asChild requires a single React element child",
        );
      }

      const childRef = (
        child as React.ReactElement & { ref?: React.Ref<HTMLElement> }
      ).ref;
      const childOnClick = child.props.onClick;
      const childCanReceiveDisabled = canReceiveDisabledAttribute(child.type);
      const iconOnly = isIconOnly(child.props.children);

      const handleClick: React.MouseEventHandler<HTMLElement> = (event) => {
        if (isDisabled) {
          event.preventDefault();
          event.stopPropagation();
          return;
        }

        childOnClick?.(event);

        if (!event.defaultPrevented) {
          onClick?.(event as React.MouseEvent<HTMLButtonElement>);
        }
      };

      const childProps: ButtonChildProps & React.RefAttributes<HTMLElement> = {
        ...props,
        "aria-busy": loading ? true : props["aria-busy"],
        children: renderContent(child.props.children),
        className: classNames(
          getButtonClassName(iconOnly),
          child.props.className,
        ),
        onClick: handleClick,
        ref: composeRefs(childRef, ref as React.Ref<HTMLElement>),
      };

      if (childCanReceiveDisabled && isDisabled) {
        childProps.disabled = true;
      } else if (isDisabled) {
        childProps["aria-disabled"] = true;
        childProps.tabIndex = -1;
      }

      if (child.type === "button") {
        childProps.type = type;
      }

      return React.cloneElement(child, childProps);
    }

    const iconOnly = isIconOnly(children);

    return (
      <button
        {...props}
        ref={ref}
        aria-busy={loading ? true : props["aria-busy"]}
        className={getButtonClassName(iconOnly)}
        disabled={isDisabled}
        onClick={onClick}
        type={type}
      >
        {renderContent(children)}
      </button>
    );
  },
);
Button.displayName = "Button";

export function ButtonGroup({
  children,
  className,
  ...props
}: ButtonGroupProps) {
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
          styles[`variant-${normalizedVariant}`],
          styles[`size-${normalizedSize}`],
          "rf-pressable",
          className,
        )}
        disabled={isDisabled}
        type={type}
      >
        <span
          aria-hidden="true"
          className={classNames(styles.icon, loading && styles.spinner)}
        >
          <Icon
            icon={loading ? LoaderCircle : icon}
            size={iconSize}
            tone={iconTone}
          />
        </span>
      </button>
    );
  },
);
IconButton.displayName = "IconButton";
