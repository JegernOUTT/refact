import React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import classNames from "classnames";

import { Portal } from "../../Portal";
import { overlayStyle } from "../overlayTypes";
import type {
  ModalOverlayContentProps,
  ModalOverlayProps,
  OverlaySide,
} from "../overlayTypes";
import styles from "./Sheet.module.css";

export type SheetSide = OverlaySide;
export type SheetProps = ModalOverlayProps;
export type SheetTriggerProps = DialogPrimitive.DialogTriggerProps;
export type SheetCloseProps = DialogPrimitive.DialogCloseProps;
export interface SheetContentProps extends ModalOverlayContentProps {
  side?: SheetSide;
}
export type SheetTitleProps = DialogPrimitive.DialogTitleProps;
export type SheetDescriptionProps = DialogPrimitive.DialogDescriptionProps;

const SheetRoot: React.FC<SheetProps> = ({ modal = true, ...props }) => {
  return <DialogPrimitive.Root modal={modal} {...props} />;
};

const SheetTrigger = DialogPrimitive.Trigger;
const SheetClose = DialogPrimitive.Close;

const SheetContent = React.forwardRef<HTMLDivElement, SheetContentProps>(
  ({ className, maxWidth, maxHeight, side = "bottom", children }, ref) => {
    return (
      <DialogPrimitive.Portal container={document.body}>
        <Portal>
          <DialogPrimitive.Overlay className={styles.overlay} />
        </Portal>
        <Portal>
          <DialogPrimitive.Content
            ref={ref}
            className={classNames(styles.content, styles[side], "rf-popover-motion", className)}
            style={overlayStyle(maxWidth, maxHeight)}
          >
            <div className={styles.inner}>{children}</div>
          </DialogPrimitive.Content>
        </Portal>
      </DialogPrimitive.Portal>
    );
  },
);

const SheetTitle = React.forwardRef<HTMLHeadingElement, SheetTitleProps>(
  ({ className, ...props }, ref) => {
    return (
      <DialogPrimitive.Title
        ref={ref}
        className={classNames(styles.title, className)}
        {...props}
      />
    );
  },
);

const SheetDescription = React.forwardRef<
  HTMLParagraphElement,
  SheetDescriptionProps
>(({ className, ...props }, ref) => {
  return (
    <DialogPrimitive.Description
      ref={ref}
      className={classNames(styles.description, className)}
      {...props}
    />
  );
});

SheetContent.displayName = "Sheet.Content";
SheetTitle.displayName = "Sheet.Title";
SheetDescription.displayName = "Sheet.Description";

export const Sheet = Object.assign(SheetRoot, {
  Trigger: SheetTrigger,
  Content: SheetContent,
  Title: SheetTitle,
  Description: SheetDescription,
  Close: SheetClose,
});
