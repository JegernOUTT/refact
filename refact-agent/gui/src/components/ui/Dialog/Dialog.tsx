import React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import classNames from "classnames";

import { Portal } from "../../Portal";
import { ModalOverlayProvider } from "../ModalOverlayContext";
import { overlayStyle } from "../overlayTypes";
import type {
  ModalOverlayContentProps,
  ModalOverlayProps,
} from "../overlayTypes";
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

const isElementOfType = (
  node: React.ReactNode,
  types: readonly React.ElementType[],
) =>
  React.isValidElement(node) && types.includes(node.type as React.ElementType);

/**
 * Splits children into a pinned header (leading Title/Description run), a
 * scrollable body, and a pinned footer (trailing Dialog.Close run). Only
 * prefix/suffix runs are lifted, so document order is always preserved and
 * consumers that interleave their own markup keep rendering unchanged.
 */
const partitionDialogChildren = (children: React.ReactNode) => {
  const nodes = React.Children.toArray(children);

  let headerEnd = 0;
  while (
    headerEnd < nodes.length &&
    isElementOfType(nodes[headerEnd], [DialogTitle, DialogDescription])
  ) {
    headerEnd += 1;
  }

  let footerStart = nodes.length;
  while (
    footerStart > headerEnd &&
    isElementOfType(nodes[footerStart - 1], [DialogClose])
  ) {
    footerStart -= 1;
  }

  return {
    header: nodes.slice(0, headerEnd),
    body: nodes.slice(headerEnd, footerStart),
    footer: nodes.slice(footerStart),
  };
};

const DialogContent = React.forwardRef<HTMLDivElement, DialogContentProps>(
  ({ className, maxWidth, maxHeight, children }, ref) => {
    const { header, body, footer } = partitionDialogChildren(children);

    return (
      <DialogPrimitive.Portal container={document.body}>
        <Portal>
          <DialogPrimitive.Overlay className={styles.overlay} />
        </Portal>
        <Portal>
          <DialogPrimitive.Content
            ref={ref}
            className={classNames(
              styles.content,
              "rf-popover-motion",
              className,
            )}
            style={overlayStyle(maxWidth, maxHeight)}
          >
            <ModalOverlayProvider value>
              {header.length > 0 ? (
                <div className={styles.header}>{header}</div>
              ) : null}
              <div className={styles.inner}>{body}</div>
              {footer.length > 0 ? (
                <div className={styles.footer}>{footer}</div>
              ) : null}
            </ModalOverlayProvider>
          </DialogPrimitive.Content>
        </Portal>
      </DialogPrimitive.Portal>
    );
  },
);

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
