import React, { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { MoreHorizontal } from "lucide-react";
import { Button, Popover } from "../ui";
import styles from "./ComposerToolbar.module.css";

export type ComposerToolbarItem = {
  id: string;
  content: React.ReactNode;
  width: number;
};

export function ComposerToolbar({
  items,
  send,
  onOpenChange,
  menuCount,
}: {
  items: ComposerToolbarItem[];
  send: React.ReactNode;
  onOpenChange: (open: boolean) => void;
  menuCount: number;
}) {
  const root = useRef<HTMLDivElement>(null);
  const inline = useRef<HTMLDivElement>(null);
  const sendRef = useRef<HTMLSpanElement>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const overflow = useRef<HTMLDivElement | null>(null);
  const hosts = useRef(new Map<string, HTMLDivElement>());
  const measuredWidths = useRef(new Map<string, number>());
  const [revision, setRevision] = useState(0);
  const [open, setOpen] = useState(false);
  const [visible, setVisible] = useState(0);
  const pendingFocus = useRef<HTMLElement | null>(null);
  const openingFocus = useRef<HTMLElement | null>(null);
  const nestedWasOpen = useRef(false);
  const nestedTrigger = useRef<HTMLElement | null>(null);
  const restoringNested = useRef(false);
  const resizeFrame = useRef<number | null>(null);

  for (const item of items) {
    if (!hosts.current.has(item.id)) {
      const host = document.createElement("div");
      host.className = styles.item;
      host.dataset.composerControl = item.id;
      hosts.current.set(item.id, host);
    }
  }

  const changeOpen = (next: boolean) => {
    if (!next && menuCount > (open ? 1 : 0)) return;
    if (next === open) return;
    setOpen(next);
    onOpenChange(next);
    if (!next && contentRef.current?.contains(document.activeElement))
      moreRef.current?.focus();
  };

  useLayoutEffect(() => {
    if (!root.current) return;
    const observer = new ResizeObserver(() =>
      setRevision((value) => value + 1),
    );
    observer.observe(root.current);
    if (sendRef.current) observer.observe(sendRef.current);
    hosts.current.forEach((host) => observer.observe(host));
    return () => {
      observer.disconnect();
      if (resizeFrame.current !== null)
        cancelAnimationFrame(resizeFrame.current);
    };
  }, []);

  useLayoutEffect(() => {
    if (!root.current) return;
    if (menuCount > (open ? 1 : 0)) {
      nestedWasOpen.current = true;
      return;
    }
    if (nestedWasOpen.current) {
      nestedWasOpen.current = false;
      restoringNested.current = true;
      resizeFrame.current = requestAnimationFrame(() => {
        if (
          document.activeElement === document.body &&
          nestedTrigger.current?.isConnected
        )
          nestedTrigger.current.focus({ preventScroll: true });
        setRevision((value) => value + 1);
      });
      return;
    }
    items.forEach((item) => {
      const host = hosts.current.get(item.id);
      if (host?.isConnected && host.parentElement === inline.current) {
        measuredWidths.current.set(item.id, host.getBoundingClientRect().width);
      }
    });
    const style = getComputedStyle(root.current);
    const gap = parseFloat(style.columnGap) || 8;
    const padding =
      (parseFloat(style.paddingLeft) || 12) +
      (parseFloat(style.paddingRight) || 12);
    let available =
      root.current.clientWidth -
      padding -
      Math.max(110, sendRef.current?.getBoundingClientRect().width ?? 0) -
      Math.max(72, moreRef.current?.getBoundingClientRect().width ?? 0) -
      gap * 2;
    let count = 0;
    for (const item of items) {
      available -=
        Math.max(item.width, measuredWidths.current.get(item.id) ?? 0) + gap;
      if (available < 0) break;
      count++;
    }
    if (!open && count < visible) {
      const focused = restoringNested.current
        ? nestedTrigger.current
        : document.activeElement;
      if (
        focused instanceof HTMLElement &&
        items
          .slice(count, visible)
          .some((item) => hosts.current.get(item.id)?.contains(focused))
      ) {
        pendingFocus.current = focused;
        openingFocus.current = focused;
        setOpen(true);
        onOpenChange(true);
      }
    }
    if (restoringNested.current && nestedTrigger.current)
      pendingFocus.current = nestedTrigger.current;
    restoringNested.current = false;
    setVisible(count);
  }, [revision, items, menuCount, open, visible, onOpenChange]);

  useLayoutEffect(() => {
    items.forEach((item, index) => {
      const host = hosts.current.get(item.id);
      if (!host) return;
      const destination = index < visible ? inline.current : overflow.current;
      if (destination) {
        if (host.parentElement !== destination) {
          const active = document.activeElement;
          destination.appendChild(host);
          if (active instanceof HTMLElement && host.contains(active))
            active.focus({ preventScroll: true });
        }
      } else host.remove();
    });
    if (pendingFocus.current?.isConnected && !open) {
      pendingFocus.current.focus({ preventScroll: true });
      pendingFocus.current = null;
    }
  });

  useLayoutEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => {
      if (openingFocus.current?.isConnected) {
        openingFocus.current.focus({ preventScroll: true });
        openingFocus.current = null;
        pendingFocus.current = null;
      } else if (!overflow.current?.contains(document.activeElement))
        overflow.current
          ?.querySelector<HTMLElement>(
            'button:not(:disabled), input:not(:disabled), [tabindex="0"]',
          )
          ?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [open]);

  return (
    <div
      className={styles.toolbar}
      ref={root}
      data-testid="composer-toolbar"
      onFocusCapture={(event) => {
        const target: EventTarget = event.target;
        if (
          target instanceof HTMLElement &&
          target.matches('[aria-haspopup="dialog"]') &&
          target !== moreRef.current
        )
          nestedTrigger.current = target;
      }}
      onPointerDownCapture={(event) => {
        const target =
          event.target instanceof Element
            ? event.target.closest<HTMLElement>('[aria-haspopup="dialog"]')
            : null;
        if (target && target !== moreRef.current)
          nestedTrigger.current = target;
      }}
    >
      <div className={styles.inline} ref={inline} />
      <Popover open={open} onOpenChange={changeOpen} responsive={false}>
        <Popover.Trigger asChild>
          <Button
            ref={moreRef}
            type="button"
            variant="soft"
            size="sm"
            leftIcon={MoreHorizontal}
            aria-label="More composer controls"
          >
            More
          </Button>
        </Popover.Trigger>
        <Popover.Content
          forceMount
          className={styles.popover}
          ref={contentRef}
          side="top"
          align="end"
          maxWidth="min(360px, calc(100vw - 24px))"
          maxHeight="min(480px, 70dvh)"
          onInteractOutside={(event) => {
            const target = event.target;
            if (target instanceof Node && contentRef.current?.contains(target))
              event.preventDefault();
          }}
        >
          <div
            className={styles.overflow}
            ref={(node) => {
              overflow.current = node;
              if (node)
                items.slice(visible).forEach((item) => {
                  const host = hosts.current.get(item.id);
                  if (host && host.parentElement !== node)
                    node.appendChild(host);
                });
            }}
          >
            <span className={styles.heading}>Composer controls</span>
          </div>
          {items.map((item) => {
            const host = hosts.current.get(item.id);
            return host ? createPortal(item.content, host, item.id) : null;
          })}
        </Popover.Content>
      </Popover>
      <span
        className={styles.send}
        ref={sendRef}
        data-composer-no-expand="true"
      >
        {send}
      </span>
    </div>
  );
}
