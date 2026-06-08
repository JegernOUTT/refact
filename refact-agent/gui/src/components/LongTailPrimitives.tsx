/* eslint-disable react/prop-types */
import React from "react";
import classNames from "classnames";
import { Badge as KitBadge } from "./ui/Badge";
import { Spinner as KitSpinner } from "./ui/Spinner";
import { Button as KitButton } from "./ui/Button";
import { Popover as KitPopover } from "./ui/Popover";
import { Tooltip as KitTooltip } from "./ui/Tooltip";
import { Surface } from "./ui/Surface";
import { Tabs as KitTabs } from "./ui/Tabs";
import { ScrollArea } from "./ScrollArea";
import styles from "./LongTailPrimitives.module.css";

// Capture the original kit sub-components BEFORE building compat namespaces.
// IMPORTANT: never mutate the shared kit components (e.g. Object.assign(KitPopover, ...)),
// and never let a compat Content render the (possibly reassigned) KitPopover.Content —
// that creates infinite render recursion and corrupts the kit globally.
const KitPopoverContent = KitPopover.Content;
const KitPopoverTrigger = KitPopover.Trigger;
const KitPopoverClose = KitPopover.Close;
const KitTooltipContent = KitTooltip.Content;
const KitTooltipTrigger = KitTooltip.Trigger;
const KitTabsList = KitTabs.List;

type Space = string | number | Record<string, string>;

type CommonProps = React.HTMLAttributes<HTMLElement> & {
  as?: React.ElementType;
  p?: Space;
  pt?: Space;
  pb?: Space;
  pl?: Space;
  px?: Space;
  py?: Space;
  mt?: Space;
  my?: Space;
  mb?: Space;
  flexGrow?: string;
  maxHeight?: string;
  maxWidth?: string;
  minWidth?: string;
  width?: string;
  size?: string;
  variant?: string;
  color?: string;
  align?: string;
  avoidCollisions?: boolean;
  hideWhenDetached?: boolean;
};

type FlexProps = CommonProps & {
  direction?: "row" | "column";
  align?: "start" | "center" | "end" | "stretch";
  justify?: "start" | "center" | "end" | "between";
  gap?: Space;
  wrap?: "nowrap" | "wrap";
};

type TextProps = CommonProps & {
  weight?: "bold" | "medium" | "regular";
  wrap?: "nowrap";
};

const space = (value: Space | undefined): string | undefined => {
  if (value == null) return undefined;
  if (typeof value === "object") return undefined;
  const normalized = String(value);
  if (normalized.endsWith("px") || normalized.includes("var(")) return normalized;
  return `var(--rf-space-${normalized})`;
};

export const Box = React.forwardRef<HTMLDivElement, CommonProps>(
  (
    {
      as,
      className,
      style,
      p,
      pt,
      pb,
      pl,
      px,
      py,
      mt,
      my,
      mb,
      flexGrow,
      maxHeight,
      maxWidth,
      minWidth,
      width,
      size: _size,
      variant: _variant,
      color: _color,
      ...props
    },
    ref,
  ) => {
    const Component = as ?? "div";
    return (
      <Component
        {...props}
        ref={ref}
        className={className}
        style={{
          padding: space(p),
          paddingTop: space(pt ?? py),
          paddingBottom: space(pb ?? py),
          paddingLeft: space(pl),
          paddingInline: space(px),
          marginTop: space(mt ?? my),
          marginBottom: space(mb ?? my),
          flexGrow: flexGrow ? Number(flexGrow) : undefined,
          maxHeight,
          maxWidth,
          minWidth,
          width,
          ...style,
        }}
      />
    );
  },
);
Box.displayName = "Box";

export const Flex = React.forwardRef<HTMLDivElement, FlexProps>(
  ({ className, style, direction = "row", align, justify, gap, wrap, ...props }, ref) => {
    return (
      <Box
        {...props}
        ref={ref}
        className={classNames(styles.flex, className)}
        style={{
          flexDirection: direction,
          alignItems: align,
          justifyContent: justify === "between" ? "space-between" : justify,
          gap: space(gap),
          flexWrap: wrap,
          ...style,
        }}
      />
    );
  },
);
Flex.displayName = "Flex";

export function Text({
  as = "span",
  className,
  size,
  weight,
  color,
  wrap,
  style,
  mb,
  ...props
}: TextProps) {
  const Component = as;
  return (
    <Component
      {...props}
      className={classNames(
        styles.text,
        size === "1" && styles.text1,
        size === "2" && styles.text2,
        weight === "bold" && styles.bold,
        weight === "medium" && styles.medium,
        color === "gray" && styles.muted,
        wrap === "nowrap" && styles.nowrap,
        className,
      )}
      style={{ marginBottom: space(mb), ...style }}
    />
  );
}

export function Heading({ as = "h4", className, ...props }: CommonProps) {
  const Component = as;
  return <Component {...props} className={classNames(styles.heading, className)} />;
}

export function Code({ className, size: _size, variant: _variant, ...props }: CommonProps) {
  return <code {...props} className={classNames(styles.code, className)} />;
}

export function Link({ className, size: _size, ...props }: CommonProps) {
  return <span {...props} className={classNames(styles.link, className)} />;
}

export function Separator({ size: _size }: { size?: string }) {
  return <div className={styles.separator} />;
}

export const Card = React.forwardRef<HTMLDivElement, CommonProps>(
  ({ className, size: _size, variant: _variant, mt, ...props }, ref) => (
    <Surface
      {...props}
      ref={ref}
      variant="surface-1"
      radius="control"
      className={className}
      style={{ marginTop: space(mt), ...props.style }}
    />
  ),
);
Card.displayName = "Card";

export function Badge({ color, variant: _variant, size: _size, ...props }: CommonProps) {
  const tone =
    color === "red" || color === "tomato"
      ? "danger"
      : color === "amber"
        ? "warning"
        : color === "green"
          ? "success"
          : color === "blue"
            ? "accent"
            : "muted";
  return <KitBadge {...props} tone={tone} />;
}

export function Button({ variant, color, size, ...props }: CommonProps & { disabled?: boolean }) {
  const kitVariant =
    color === "red"
      ? "danger"
      : variant === "solid"
        ? "primary"
        : variant === "surface" || variant === "outline"
          ? "soft"
          : variant === "ghost" || variant === "soft" || variant === "plain"
            ? variant
            : "ghost";
  const kitSize = size === "1" ? "sm" : size === "3" ? "lg" : "md";
  return <KitButton {...props} variant={kitVariant} size={kitSize} />;
}

type PopoverContentCompatProps = Omit<CommonProps, "align"> &
  React.ComponentProps<typeof KitPopover.Content> & {
    avoidCollisions?: boolean;
    hideWhenDetached?: boolean;
  };

function PopoverContent({
  width,
  minWidth,
  maxWidth,
  maxHeight,
  style: _style,
  size: _size,
  avoidCollisions: _avoidCollisions,
  hideWhenDetached: _hideWhenDetached,
  ...props
}: PopoverContentCompatProps) {
  return (
    <KitPopoverContent
      {...props}
      maxWidth={maxWidth ?? width ?? minWidth}
      maxHeight={maxHeight}
    />
  );
}

const PopoverRootCompat = (props: React.ComponentProps<typeof KitPopover>) => (
  <KitPopover {...props} />
);

export const Popover = Object.assign(PopoverRootCompat, {
  Root: KitPopover,
  Trigger: KitPopoverTrigger,
  Content: PopoverContent,
  Close: KitPopoverClose,
});

type HoverCardProps = React.ComponentProps<typeof KitTooltip> & {
  openDelay?: number;
  closeDelay?: number;
};

type HoverCardContentCompatProps = Omit<CommonProps, "align"> &
  React.ComponentProps<typeof KitTooltip.Content> & {
    avoidCollisions?: boolean;
    hideWhenDetached?: boolean;
  };

function HoverCardRoot({ openDelay, closeDelay, ...props }: HoverCardProps) {
  return (
    <KitTooltip
      {...props}
      delayDuration={openDelay}
      skipDelayDuration={closeDelay}
    />
  );
}

function HoverCardContent({
  width,
  minWidth,
  maxWidth,
  maxHeight,
  style: _style,
  size: _size,
  avoidCollisions: _avoidCollisions,
  hideWhenDetached: _hideWhenDetached,
  ...props
}: HoverCardContentCompatProps) {
  return (
    <KitTooltipContent
      {...props}
      maxWidth={maxWidth ?? width ?? minWidth}
      maxHeight={maxHeight}
    />
  );
}

export const HoverCard = Object.assign(HoverCardRoot, {
  Root: HoverCardRoot,
  Trigger: KitTooltipTrigger,
  Content: HoverCardContent,
});

function TabsList({ size: _size, ...props }: React.ComponentProps<typeof KitTabs.List> & { size?: string }) {
  return <KitTabsList {...props} />;
}

const TabsRootCompat = (props: React.ComponentProps<typeof KitTabs>) => (
  <KitTabs {...props} />
);

export const Tabs = Object.assign(TabsRootCompat, {
  Root: KitTabs,
  List: TabsList,
  Trigger: KitTabs.Trigger,
  Content: KitTabs.Content,
});

export function Checkbox({ checked, onCheckedChange, disabled }: {
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <input
      type="checkbox"
      checked={checked}
      disabled={disabled}
      onChange={(event) => onCheckedChange?.(event.currentTarget.checked)}
    />
  );
}

export function Spinner({ size: _size }: { size?: string }) {
  return <KitSpinner />;
}

export { ScrollArea };
