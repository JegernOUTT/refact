import React from "react";
import classNames from "classnames";
import { LoaderCircle } from "lucide-react";
import {
  Badge as KitBadge,
  Icon,
  Skeleton as KitSkeleton,
  Tooltip as KitTooltip,
} from "../../../../components/ui";
import styles from "./DashboardPrimitives.module.css";

export interface DashboardTextProps extends React.ComponentProps<"span"> {
  size?: "1" | "2" | "3";
  weight?: "medium" | "bold";
  tone?: "default" | "muted" | "danger" | "success" | "warning" | "accent";
  color?: "red" | "amber" | "green" | "blue";
  truncate?: boolean;
}

export function DashboardText({
  children,
  className,
  color,
  size = "2",
  weight,
  tone = "default",
  truncate = false,
  ...props
}: DashboardTextProps) {
  const resolvedTone =
    color === "red"
      ? "danger"
      : color === "amber"
        ? "warning"
        : color === "green"
          ? "success"
          : color === "blue"
            ? "accent"
            : tone;

  return (
    <span
      {...props}
      className={classNames(
        styles.text,
        size === "1" ? styles.size1 : styles.size2,
        weight === "medium" && styles.weightMedium,
        weight === "bold" && styles.weightBold,
        resolvedTone !== "default" && styles[resolvedTone],
        truncate && styles.truncate,
        className,
      )}
    >
      {children}
    </span>
  );
}

export interface DashboardFlexProps extends React.ComponentProps<"div"> {
  direction?: "row" | "column";
  gap?: "1" | "2" | "3";
  align?: "center";
  justify?: "center" | "between";
  py?: "2";
  mb?: string;
}

export function DashboardFlex({
  children,
  className,
  direction = "row",
  gap,
  align,
  justify,
  mb,
  py,
  ...props
}: DashboardFlexProps) {
  return (
    <div
      {...props}
      className={classNames(styles.flex, className)}
      data-align={align}
      data-direction={direction}
      data-gap={gap}
      data-justify={justify}
      data-mb={mb}
      data-py={py}
    >
      {children}
    </div>
  );
}

interface DashboardTextFieldContextValue {
  size?: "1";
}

const DashboardTextFieldContext =
  React.createContext<DashboardTextFieldContextValue>({});

interface DashboardTextFieldRootProps
  extends Omit<React.ComponentProps<"div">, "onChange"> {
  value: string;
  placeholder?: string;
  onChange: React.ChangeEventHandler<HTMLInputElement>;
  onKeyDown?: React.KeyboardEventHandler<HTMLInputElement>;
  autoFocus?: boolean;
  size?: "1";
}

function DashboardTextFieldRoot({
  autoFocus,
  children,
  className,
  onChange,
  onKeyDown,
  placeholder,
  size,
  value,
  ...props
}: DashboardTextFieldRootProps) {
  return (
    <DashboardTextFieldContext.Provider value={{ size }}>
      <div {...props} className={classNames(styles.textField, className)}>
        {children}
        <input
          autoFocus={autoFocus}
          className={styles.textFieldInput}
          onChange={onChange}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          value={value}
        />
      </div>
    </DashboardTextFieldContext.Provider>
  );
}

interface DashboardTextFieldSlotProps extends React.ComponentProps<"span"> {
  className?: string;
}

function DashboardTextFieldSlot({
  children,
  className,
  ...rest
}: DashboardTextFieldSlotProps) {
  React.useContext(DashboardTextFieldContext);
  return (
    <span {...rest} className={classNames(styles.textFieldSlot, className)}>
      {children}
    </span>
  );
}

export const DashboardTextField = Object.assign(DashboardTextFieldRoot, {
  Root: DashboardTextFieldRoot,
  Slot: DashboardTextFieldSlot,
});

export function DashboardSpinner() {
  return (
    <span className={classNames(styles.spinner, "rf-spin")}>
      <Icon icon={LoaderCircle} size="md" tone="muted" />
    </span>
  );
}

interface DashboardHoverCardProps extends React.ComponentProps<"span"> {
  children: React.ReactNode;
  openDelay?: number;
  closeDelay?: number;
  side?: string;
  align?: string;
  size?: string;
  avoidCollisions?: boolean;
}

function DashboardHoverCardRoot({ children }: DashboardHoverCardProps) {
  return <span className={styles.hoverRoot}>{children}</span>;
}

function DashboardHoverCardTrigger({ children }: DashboardHoverCardProps) {
  return <>{children}</>;
}

function DashboardHoverCardContent({
  avoidCollisions,
  align,
  children,
  className,
  side,
  size,
  ...props
}: DashboardHoverCardProps & { className?: string }) {
  void avoidCollisions;
  void align;
  void children;
  void className;
  void props;
  void side;
  void size;
  return null;
}
// eslint-disable-next-line react-refresh/only-export-components
export const DashboardHoverCard = {
  Root: DashboardHoverCardRoot,
  Trigger: DashboardHoverCardTrigger,
  Content: DashboardHoverCardContent,
};

export interface DashboardBadgeProps
  extends React.ComponentProps<typeof KitBadge> {
  color?: string;
  variant?: string;
  size?: string;
}

export function DashboardBadge({
  color,
  size,
  variant,
  tone,
  ...props
}: DashboardBadgeProps) {
  void color;
  void size;
  void variant;
  return <KitBadge tone={tone ?? "accent"} {...props} />;
}

export function DashboardTooltip({
  content,
  children,
}: {
  content: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <KitTooltip>
      <KitTooltip.Trigger asChild>{children}</KitTooltip.Trigger>
      <KitTooltip.Content>{content}</KitTooltip.Content>
    </KitTooltip>
  );
}

export function DashboardSkeleton({
  children,
  className,
  height,
  width,
}: {
  children?: React.ReactNode;
  className?: string;
  height?: string;
  width?: string;
}) {
  if (children) return <>{children}</>;
  return <KitSkeleton className={className} height={height} width={width} />;
}
