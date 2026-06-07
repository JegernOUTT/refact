import type React from "react";

export type OverlaySide = "top" | "right" | "bottom" | "left";
export type OverlayAlign = "start" | "center" | "end";

export interface OverlayRootProps {
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children?: React.ReactNode;
}

export interface AnchoredOverlayContentProps {
  side?: OverlaySide;
  align?: OverlayAlign;
  sideOffset?: number;
  collisionPadding?: number;
  maxWidth?: string;
  maxHeight?: string;
  className?: string;
  children: React.ReactNode;
}

export interface ModalOverlayProps extends OverlayRootProps {
  modal?: boolean;
}

export interface ModalOverlayContentProps {
  maxWidth?: string;
  maxHeight?: string;
  className?: string;
  children: React.ReactNode;
}

export const overlayStyle = (
  maxWidth?: string,
  maxHeight?: string,
): React.CSSProperties => {
  return {
    "--rf-overlay-max-width": maxWidth,
    "--rf-overlay-max-height": maxHeight,
  } as React.CSSProperties;
};
