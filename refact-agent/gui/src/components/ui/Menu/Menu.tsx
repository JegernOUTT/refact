import React from "react";
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import classNames from "classnames";

import { Portal } from "../../Portal";
import { overlayStyle } from "../overlayTypes";
import type {
  AnchoredOverlayContentProps,
  OverlayRootProps,
} from "../overlayTypes";
import styles from "./Menu.module.css";

export interface MenuProps extends OverlayRootProps {
  modal?: boolean;
}
export type MenuTriggerProps = DropdownMenuPrimitive.DropdownMenuTriggerProps;
export type MenuContentProps = AnchoredOverlayContentProps;
export type MenuItemProps = DropdownMenuPrimitive.DropdownMenuItemProps;
export type MenuLabelProps = DropdownMenuPrimitive.DropdownMenuLabelProps;
export type MenuSeparatorProps =
  DropdownMenuPrimitive.DropdownMenuSeparatorProps;

const MenuRoot: React.FC<MenuProps> = ({ modal = true, ...props }) => {
  return <DropdownMenuPrimitive.Root modal={modal} {...props} />;
};

const MenuTrigger = DropdownMenuPrimitive.Trigger;

const MenuContent = React.forwardRef<HTMLDivElement, MenuContentProps>(
  (
    {
      className,
      maxWidth,
      maxHeight,
      side = "bottom",
      align = "start",
      sideOffset = 8,
      collisionPadding = 12,
      children,
    },
    ref,
  ) => {
    return (
      <DropdownMenuPrimitive.Portal container={document.body}>
        <Portal>
          <DropdownMenuPrimitive.Content
            ref={ref}
            side={side}
            align={align}
            sideOffset={sideOffset}
            collisionPadding={collisionPadding}
            className={classNames(
              styles.content,
              "rf-popover-motion",
              className,
            )}
            style={overlayStyle(maxWidth, maxHeight)}
          >
            {children}
          </DropdownMenuPrimitive.Content>
        </Portal>
      </DropdownMenuPrimitive.Portal>
    );
  },
);

const MenuItem = React.forwardRef<HTMLDivElement, MenuItemProps>(
  ({ className, ...props }, ref) => {
    return (
      <DropdownMenuPrimitive.Item
        ref={ref}
        className={classNames(styles.item, className)}
        {...props}
      />
    );
  },
);

const MenuLabel = React.forwardRef<HTMLDivElement, MenuLabelProps>(
  ({ className, ...props }, ref) => {
    return (
      <DropdownMenuPrimitive.Label
        ref={ref}
        className={classNames(styles.label, className)}
        {...props}
      />
    );
  },
);

const MenuSeparator = React.forwardRef<HTMLDivElement, MenuSeparatorProps>(
  ({ className, ...props }, ref) => {
    return (
      <DropdownMenuPrimitive.Separator
        ref={ref}
        className={classNames(styles.separator, className)}
        {...props}
      />
    );
  },
);

MenuContent.displayName = "Menu.Content";
MenuItem.displayName = "Menu.Item";
MenuLabel.displayName = "Menu.Label";
MenuSeparator.displayName = "Menu.Separator";

export const Menu = Object.assign(MenuRoot, {
  Trigger: MenuTrigger,
  Content: MenuContent,
  Item: MenuItem,
  Label: MenuLabel,
  Separator: MenuSeparator,
});
