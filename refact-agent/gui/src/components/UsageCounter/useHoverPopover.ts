import React, { useCallback, useEffect, useRef, useState } from "react";

export const HOVER_OPEN_DELAY_MS = 200;
export const HOVER_CLOSE_DELAY_MS = 150;

type OpenSource = "hover" | "click";

export function useHoverPopover(onHoldChange?: (held: boolean) => void) {
  const [open, setOpen] = useState(false);
  const sourceRef = useRef<OpenSource | null>(null);
  const closedSourceRef = useRef<OpenSource | null>(null);
  const openTimer = useRef<number>();
  const closeTimer = useRef<number>();
  const pointerInsideRef = useRef(false);
  const focusHeldRef = useRef(false);
  const restoreFocusRef = useRef(false);
  const triggerElRef = useRef<HTMLButtonElement | null>(null);
  const contentElRef = useRef<HTMLDivElement | null>(null);

  const clearTimers = useCallback(() => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
    openTimer.current = undefined;
    closeTimer.current = undefined;
  }, []);

  const update = useCallback(
    (next: boolean, source: OpenSource) => {
      clearTimers();
      if (next) {
        sourceRef.current = source;
      } else {
        const content = contentElRef.current;
        restoreFocusRef.current = Boolean(
          content?.contains(document.activeElement),
        );
        focusHeldRef.current = false;
        closedSourceRef.current = sourceRef.current;
        sourceRef.current = null;
      }
      setOpen(next);
    },
    [clearTimers],
  );

  useEffect(() => clearTimers, [clearTimers]);

  useEffect(() => {
    if (!open) return;
    onHoldChange?.(true);
    return () => onHoldChange?.(false);
  }, [open, onHoldChange]);

  useEffect(() => {
    if (open || !restoreFocusRef.current) return;
    const active = document.activeElement;
    if (!active || active === document.body) triggerElRef.current?.focus();
  }, [open]);

  const onOpenChange = useCallback(
    (next: boolean) => update(next, "click"),
    [update],
  );

  const scheduleClose = useCallback(() => {
    window.clearTimeout(openTimer.current);
    openTimer.current = undefined;
    if (sourceRef.current !== "hover" || focusHeldRef.current) return;
    window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(
      () => update(false, "hover"),
      HOVER_CLOSE_DELAY_MS,
    );
  }, [update]);

  const handleEnter = useCallback(
    (event: React.PointerEvent) => {
      if (event.pointerType === "touch") return;
      pointerInsideRef.current = true;
      window.clearTimeout(closeTimer.current);
      closeTimer.current = undefined;
      if (sourceRef.current !== null || openTimer.current !== undefined) return;
      openTimer.current = window.setTimeout(
        () => update(true, "hover"),
        HOVER_OPEN_DELAY_MS,
      );
    },
    [update],
  );

  const handleLeave = useCallback(() => {
    pointerInsideRef.current = false;
    scheduleClose();
  }, [scheduleClose]);

  const handleFocusOut = useCallback(
    (next: EventTarget | null) => {
      if (!focusHeldRef.current) return;
      const node = next instanceof Node ? next : null;
      if (
        node &&
        (node === triggerElRef.current || contentElRef.current?.contains(node))
      ) {
        return;
      }
      focusHeldRef.current = false;
      if (!pointerInsideRef.current) scheduleClose();
    },
    [scheduleClose],
  );

  const onContentFocusIn = useCallback(() => {
    focusHeldRef.current = true;
    window.clearTimeout(closeTimer.current);
    closeTimer.current = undefined;
  }, []);

  const onContentFocusOut = useCallback(
    (event: FocusEvent) => handleFocusOut(event.relatedTarget),
    [handleFocusOut],
  );

  const contentRef = useCallback(
    (node: HTMLDivElement | null) => {
      const previous = contentElRef.current;
      if (previous) {
        previous.removeEventListener("focusin", onContentFocusIn);
        previous.removeEventListener("focusout", onContentFocusOut);
      }
      contentElRef.current = node;
      if (node) {
        node.addEventListener("focusin", onContentFocusIn);
        node.addEventListener("focusout", onContentFocusOut);
      }
    },
    [onContentFocusIn, onContentFocusOut],
  );

  const triggerRef = useCallback((node: HTMLButtonElement | null) => {
    triggerElRef.current = node;
  }, []);

  const triggerProps = {
    ref: triggerRef,
    onPointerEnter: handleEnter,
    onPointerLeave: handleLeave,
    onBlur: (event: React.FocusEvent) => handleFocusOut(event.relatedTarget),
    onClick: (event: React.MouseEvent) => {
      if (sourceRef.current === "hover") {
        event.preventDefault();
        clearTimers();
        sourceRef.current = "click";
      } else if (sourceRef.current === null) {
        clearTimers();
      }
    },
  };

  const contentProps = {
    ref: contentRef,
    onPointerEnter: handleEnter,
    onPointerLeave: handleLeave,
    onPointerDown: () => {
      if (sourceRef.current !== "hover") return;
      clearTimers();
      sourceRef.current = "click";
    },
    onOpenAutoFocus: (event: Event) => {
      if (sourceRef.current === "hover") event.preventDefault();
    },
    onCloseAutoFocus: (event: Event) => {
      if (restoreFocusRef.current) {
        event.preventDefault();
        const active = document.activeElement;
        if (
          !active ||
          active === document.body ||
          contentElRef.current?.contains(active)
        ) {
          triggerElRef.current?.focus();
        }
      } else if (closedSourceRef.current === "hover") {
        event.preventDefault();
      }
      restoreFocusRef.current = false;
      closedSourceRef.current = null;
    },
  };

  return { open, onOpenChange, triggerProps, contentProps };
}
