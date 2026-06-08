import React from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import classNames from "classnames";

import { Portal } from "../../Portal";
import { overlayStyle } from "../overlayTypes";
import type {
  AnchoredOverlayContentProps,
  OverlayRootProps,
} from "../overlayTypes";
import styles from "./Tooltip.module.css";

export interface TooltipProps extends OverlayRootProps {
  delayDuration?: number;
  skipDelayDuration?: number;
  content?: React.ReactNode;
}
export type TooltipTriggerProps = TooltipPrimitive.TooltipTriggerProps;
export type TooltipContentProps = AnchoredOverlayContentProps;

const TooltipRoot: React.FC<TooltipProps> = ({
  delayDuration = 350,
  skipDelayDuration = 150,
  children,
  content,
  ...props
}) => {
  const wrappedChildren = content ? (
    <>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipContent>{content}</TooltipContent>
    </>
  ) : (
    children
  );

  return (
    <TooltipPrimitive.Provider
      delayDuration={delayDuration}
      skipDelayDuration={skipDelayDuration}
    >
      <TooltipPrimitive.Root {...props}>
        {wrappedChildren}
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
};

const TooltipTrigger = TooltipPrimitive.Trigger;

const TooltipContent = React.forwardRef<HTMLDivElement, TooltipContentProps>(
  (
    {
      className,
      maxWidth,
      maxHeight,
      side = "top",
      align = "center",
      sideOffset = 6,
      collisionPadding = 12,
      children,
    },
    ref,
  ) => {
    return (
      <TooltipPrimitive.Portal container={document.body}>
        <Portal>
          <TooltipPrimitive.Content
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
          </TooltipPrimitive.Content>
        </Portal>
      </TooltipPrimitive.Portal>
    );
  },
);

TooltipContent.displayName = "Tooltip.Content";

export const Tooltip = Object.assign(TooltipRoot, {
  Trigger: TooltipTrigger,
  Content: TooltipContent,
});
