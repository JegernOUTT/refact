import React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";
import classNames from "classnames";

import { Portal } from "../../Portal";
import { useIsInsideModalOverlay } from "../ModalOverlayContext";
import { Sheet } from "../Sheet";
import { overlayStyle } from "../overlayTypes";
import type {
  AnchoredOverlayContentProps,
  OverlayRootProps,
} from "../overlayTypes";
import { useMediaQuery } from "../useMediaQuery";
import styles from "./Popover.module.css";

const narrowQuery = "(max-width: 479px)";

type PopoverContextValue = {
  responsive: boolean;
  forceSheet: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  triggerRef: React.MutableRefObject<HTMLButtonElement | null>;
};

const PopoverContext = React.createContext<PopoverContextValue | null>(null);

export interface PopoverProps extends OverlayRootProps {
  modal?: boolean;
  responsive?: boolean;
  forceSheet?: boolean;
}
export type PopoverTriggerProps = PopoverPrimitive.PopoverTriggerProps;
export interface PopoverContentProps extends AnchoredOverlayContentProps {
  scrollable?: boolean;
  style?: React.CSSProperties;
  onInteractOutside?: PopoverPrimitive.PopoverContentProps["onInteractOutside"];
  onEscapeKeyDown?: PopoverPrimitive.PopoverContentProps["onEscapeKeyDown"];
  onOpenAutoFocus?: PopoverPrimitive.PopoverContentProps["onOpenAutoFocus"];
  forceMount?: true;
}
export type PopoverCloseProps = PopoverPrimitive.PopoverCloseProps;

const useOpenState = ({
  open,
  defaultOpen = false,
  onOpenChange,
}: Pick<PopoverProps, "open" | "defaultOpen" | "onOpenChange">) => {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const actualOpen = open ?? uncontrolledOpen;

  const setActualOpen = React.useCallback(
    (nextOpen: boolean) => {
      if (open === undefined) {
        setUncontrolledOpen(nextOpen);
      }
      onOpenChange?.(nextOpen);
    },
    [onOpenChange, open],
  );

  return [actualOpen, setActualOpen] as const;
};

const PopoverRoot: React.FC<PopoverProps> = ({
  modal = false,
  responsive = true,
  forceSheet = false,
  open,
  defaultOpen,
  onOpenChange,
  children,
}) => {
  const [actualOpen, setActualOpen] = useOpenState({
    open,
    defaultOpen,
    onOpenChange,
  });
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);

  return (
    <PopoverContext.Provider
      value={{
        responsive,
        forceSheet,
        open: actualOpen,
        onOpenChange: setActualOpen,
        triggerRef,
      }}
    >
      <PopoverPrimitive.Root
        modal={modal}
        open={actualOpen}
        onOpenChange={setActualOpen}
      >
        {children}
      </PopoverPrimitive.Root>
    </PopoverContext.Provider>
  );
};

const PopoverTrigger = React.forwardRef<HTMLButtonElement, PopoverTriggerProps>(
  (props, ref) => {
    const context = React.useContext(PopoverContext);
    return (
      <PopoverPrimitive.Trigger
        {...props}
        ref={(node) => {
          if (context) context.triggerRef.current = node;
          if (typeof ref === "function") ref(node);
          else if (ref) ref.current = node;
        }}
      />
    );
  },
);
PopoverTrigger.displayName = "Popover.Trigger";
const PopoverClose = PopoverPrimitive.Close;

const PopoverContent = React.forwardRef<HTMLDivElement, PopoverContentProps>(
  (
    {
      className,
      maxWidth,
      maxHeight,
      side = "bottom",
      align = "center",
      sideOffset = 8,
      collisionPadding = 12,
      scrollable = true,
      style,
      onInteractOutside,
      onEscapeKeyDown,
      onOpenAutoFocus,
      forceMount,
      children,
    },
    ref,
  ) => {
    const context = React.useContext(PopoverContext);
    const isInsideModalOverlay = useIsInsideModalOverlay();
    const isNarrow = useMediaQuery(narrowQuery);
    const renderSheet = Boolean(
      context && (context.forceSheet || (context.responsive && isNarrow)),
    );

    if (renderSheet && context) {
      return (
        <Sheet open={context.open} onOpenChange={context.onOpenChange}>
          <Sheet.Content
            ref={ref}
            className={className}
            maxWidth={maxWidth}
            maxHeight={maxHeight}
            scrollable={scrollable}
            style={style}
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              context.triggerRef.current?.focus();
            }}
          >
            {children}
          </Sheet.Content>
        </Sheet>
      );
    }

    return (
      <PopoverPrimitive.Portal
        container={document.body}
        forceMount={forceMount}
      >
        <Portal>
          <PopoverPrimitive.Content
            forceMount={forceMount}
            ref={ref}
            side={side}
            align={align}
            sideOffset={sideOffset}
            collisionPadding={collisionPadding}
            className={classNames(
              styles.content,
              isInsideModalOverlay && styles.contentInModal,
              !scrollable && styles.contentNoScroll,
              "rf-popover-motion",
              className,
            )}
            style={{ ...overlayStyle(maxWidth, maxHeight), ...style }}
            onInteractOutside={onInteractOutside}
            onEscapeKeyDown={onEscapeKeyDown}
            onOpenAutoFocus={(event) => {
              if (forceMount) {
                event.preventDefault();
                return;
              }
              onOpenAutoFocus?.(event);
            }}
          >
            <div
              className={classNames(
                styles.inner,
                !scrollable && styles.innerNoScroll,
              )}
            >
              {children}
            </div>
          </PopoverPrimitive.Content>
        </Portal>
      </PopoverPrimitive.Portal>
    );
  },
);

PopoverContent.displayName = "Popover.Content";

export const Popover = Object.assign(PopoverRoot, {
  Trigger: PopoverTrigger,
  Content: PopoverContent,
  Close: PopoverClose,
});
