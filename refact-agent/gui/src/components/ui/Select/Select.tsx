import React from "react";
import * as SelectPrimitive from "@radix-ui/react-select";
import classNames from "classnames";
import { Check, ChevronDown } from "lucide-react";

import { Portal } from "../../Portal";
import { Icon } from "../Icon";
import { overlayStyle } from "../overlayTypes";
import type { AnchoredOverlayContentProps } from "../overlayTypes";
import styles from "./Select.module.css";

export interface SelectProps extends SelectPrimitive.SelectProps {
  children?: React.ReactNode;
}
export interface SelectTriggerProps extends SelectPrimitive.SelectTriggerProps {
  placeholder?: string;
}
export type SelectValueProps = SelectPrimitive.SelectValueProps;
export interface SelectContentProps extends AnchoredOverlayContentProps {
  position?: SelectPrimitive.SelectContentProps["position"];
}
export type SelectItemProps = SelectPrimitive.SelectItemProps;
export type SelectGroupProps = SelectPrimitive.SelectGroupProps;
export type SelectLabelProps = SelectPrimitive.SelectLabelProps;
export type SelectSeparatorProps = SelectPrimitive.SelectSeparatorProps;

function SelectRoot({ children, ...props }: SelectProps) {
  return <SelectPrimitive.Root {...props}>{children}</SelectPrimitive.Root>;
}

const SelectValue = SelectPrimitive.Value;

const SelectTrigger = React.forwardRef<HTMLButtonElement, SelectTriggerProps>(
  ({ children, className, placeholder, ...props }, ref) => {
    return (
      <SelectPrimitive.Trigger
        {...props}
        ref={ref}
        className={classNames(styles.trigger, className)}
      >
        <SelectPrimitive.Value placeholder={placeholder}>{children}</SelectPrimitive.Value>
        <SelectPrimitive.Icon className={styles.icon}>
          <Icon icon={ChevronDown} size="sm" />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
    );
  },
);
SelectTrigger.displayName = "Select.Trigger";

const SelectContent = React.forwardRef<HTMLDivElement, SelectContentProps>(
  (
    {
      align = "start",
      children,
      className,
      collisionPadding = 12,
      maxHeight,
      maxWidth,
      position = "popper",
      side = "bottom",
      sideOffset = 8,
    },
    ref,
  ) => {
    return (
      <SelectPrimitive.Portal container={document.body}>
        <Portal>
          <SelectPrimitive.Content
            ref={ref}
            align={align}
            className={classNames(styles.content, "rf-popover-motion", className)}
            collisionPadding={collisionPadding}
            position={position}
            side={side}
            sideOffset={sideOffset}
            style={overlayStyle(maxWidth, maxHeight)}
          >
            <SelectPrimitive.Viewport className={styles.viewport}>{children}</SelectPrimitive.Viewport>
          </SelectPrimitive.Content>
        </Portal>
      </SelectPrimitive.Portal>
    );
  },
);
SelectContent.displayName = "Select.Content";

const SelectItem = React.forwardRef<HTMLDivElement, SelectItemProps>(
  ({ children, className, ...props }, ref) => {
    return (
      <SelectPrimitive.Item
        {...props}
        ref={ref}
        className={classNames(styles.item, className)}
      >
        <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
        <SelectPrimitive.ItemIndicator className={styles.itemIndicator}>
          <Icon icon={Check} size="sm" tone="accent" />
        </SelectPrimitive.ItemIndicator>
      </SelectPrimitive.Item>
    );
  },
);
SelectItem.displayName = "Select.Item";

const SelectGroup = SelectPrimitive.Group;
const SelectLabel = React.forwardRef<HTMLDivElement, SelectLabelProps>(
  ({ className, ...props }, ref) => (
    <SelectPrimitive.Label {...props} ref={ref} className={classNames(styles.label, className)} />
  ),
);
SelectLabel.displayName = "Select.Label";

const SelectSeparator = React.forwardRef<HTMLDivElement, SelectSeparatorProps>(
  ({ className, ...props }, ref) => (
    <SelectPrimitive.Separator
      {...props}
      ref={ref}
      className={classNames(styles.separator, className)}
    />
  ),
);
SelectSeparator.displayName = "Select.Separator";

export const Select = Object.assign(SelectRoot, {
  Content: SelectContent,
  Group: SelectGroup,
  Item: SelectItem,
  Label: SelectLabel,
  Separator: SelectSeparator,
  Trigger: SelectTrigger,
  Value: SelectValue,
});
