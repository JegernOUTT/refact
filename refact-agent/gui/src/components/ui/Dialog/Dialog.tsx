import React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import classNames from "classnames";

import { Portal } from "../../Portal";
import { overlayStyle } from "../overlayTypes";
import type { ModalOverlayContentProps, ModalOverlayProps } from "../overlayTypes";
import styles from "./Dialog.module.css";

export type DialogProps = ModalOverlayProps;
export type DialogTriggerProps = DialogPrimitive.DialogTriggerProps;
export type DialogCloseProps = DialogPrimitive.DialogCloseProps;
export type DialogContentProps = ModalOverlayContentProps;
export type DialogTitleProps = DialogPrimitive.DialogTitleProps;
export type DialogDescriptionProps = DialogPrimitive.DialogDescriptionProps;

const DialogRoot: React.FC<DialogProps> = ({ modal = true, ...props }) => {
  return <DialogPrimitive.Root modal={modal} {...props} />;
};

const DialogTrigger = DialogPrimitive.Trigger;
const DialogClose = DialogPrimitive.Close;

const DialogContent = React.forwardRef<HTMLDivElement, DialogContentProps>(
  ({ className, maxWidth, maxHeight, children }, ref) => {
    return (
      <DialogPrimitive.Portal container={document.body}>
        <Portal>
          <DialogPrimitive.Overlay className={styles.overlay} />
        </Portal>
        <Portal>
          <DialogPrimitive.Content
            ref={ref}
            className={classNames(styles.content, "rf-popover-motion", className)}
            style={overlayStyle(maxWidth, maxHeight)}
          >
            <div className={styles.inner}>{children}</div>
          </DialogPrimitive.Content>
        </Portal>
      </DialogPrimitive.Portal>
    );
  },
);

const DialogTitle = React.forwardRef<HTMLHeadingElement, DialogTitleProps>(
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

const DialogDescription = React.forwardRef<
  HTMLParagraphElement,
  DialogDescriptionProps
>(({ className, ...props }, ref) => {
  return (
    <DialogPrimitive.Description
      ref={ref}
      className={classNames(styles.description, className)}
      {...props}
    />
  );
});

DialogContent.displayName = "Dialog.Content";
DialogTitle.displayName = "Dialog.Title";
DialogDescription.displayName = "Dialog.Description";

export const Dialog = Object.assign(DialogRoot, {
  Trigger: DialogTrigger,
  Content: DialogContent,
  Title: DialogTitle,
  Description: DialogDescription,
  Close: DialogClose,
});
