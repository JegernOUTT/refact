import React, { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import styles from "./ComposerToolbar.module.css";

export type ComposerToolbarItem = {
  id: string;
  content: React.ReactNode;
  row: "top" | "bottom";
  priority: number;
  side?: "left" | "right";
  pinned?: boolean;
  hidden?: boolean;
};

export function ComposerToolbar({
  items,
  editor,
  menuCount,
}: {
  items: ComposerToolbarItem[];
  editor: React.ReactNode;
  menuCount: number;
}) {
  const root = useRef<HTMLDivElement>(null);
  const top = useRef<HTMLDivElement>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const hosts = useRef(new Map<string, HTMLSpanElement>());
  const [revision, setRevision] = useState(0);
  const frame = useRef<number>();

  for (const item of items) {
    if (!hosts.current.has(item.id)) {
      const host = document.createElement("span");
      host.className = styles.item;
      host.dataset.composerControl = item.id;
      hosts.current.set(item.id, host);
    }
  }

  useLayoutEffect(() => {
    const observer = new ResizeObserver(() => {
      if (frame.current !== undefined) return;
      frame.current = requestAnimationFrame(() => {
        frame.current = undefined;
        setRevision((value) => value + 1);
      });
    });
    if (root.current) observer.observe(root.current);
    hosts.current.forEach((host) => observer.observe(host));
    return () => {
      observer.disconnect();
      if (frame.current !== undefined) cancelAnimationFrame(frame.current);
    };
  }, []);

  useLayoutEffect(() => {
    const openHosts = new Set(
      items
        .filter(
          (item) =>
            hosts.current
              .get(item.id)
              ?.querySelector(
                '[aria-haspopup][aria-expanded="true"], [aria-haspopup][data-state="open"]',
              ),
        )
        .map((item) => item.id),
    );
    const rowFor = (item: ComposerToolbarItem) => {
      const parent = hosts.current.get(item.id)?.parentElement;
      if (menuCount > 0 && openHosts.has(item.id)) {
        if (parent === top.current) return "top";
        if (parent === bottom.current) return "bottom";
      }
      return item.row;
    };

    for (const [name, row] of [
      ["top", top.current],
      ["bottom", bottom.current],
    ] as const) {
      if (!row) continue;
      const controls = items.filter((item) => rowFor(item) === name);
      for (const [index, item] of controls.entries()) {
        const host = hosts.current.get(item.id);
        if (!host) continue;
        if (row.children[index] !== host) {
          const active = document.activeElement;
          row.insertBefore(host, row.children.item(index));
          if (active instanceof HTMLElement && host.contains(active))
            active.focus({ preventScroll: true });
        }
        host.dataset.side = item.side ?? "left";
        host.dataset.measuring = "true";
        host.dataset.compactContext = "false";
      }

      const style = getComputedStyle(row);
      const gap = parseFloat(style.columnGap) || 0;
      const capacity =
        (root.current?.clientWidth ?? 0) -
        (parseFloat(style.paddingLeft) || 0) -
        (parseFloat(style.paddingRight) || 0);
      const widths = new Map(
        controls.map((item) => [
          item.id,
          hosts.current.get(item.id)?.getBoundingClientRect().width ?? 0,
        ]),
      );
      const visible = controls.filter(
        (item) =>
          (!item.hidden || openHosts.has(item.id)) &&
          (widths.get(item.id) ?? 0) > 0,
      );
      const visibleIds = new Set(visible.map((item) => item.id));
      const hidden = new Set(
        controls
          .filter((item) => !visibleIds.has(item.id))
          .map((item) => item.id),
      );
      let required =
        visible.reduce((sum, item) => sum + (widths.get(item.id) ?? 0), 0) +
        Math.max(0, visible.length - 1) * gap;

      for (const item of [...visible].sort((a, b) => a.priority - b.priority)) {
        if (required <= capacity) break;
        if (item.pinned === true || openHosts.has(item.id)) continue;
        hidden.add(item.id);
        required -= (widths.get(item.id) ?? 0) + gap;
      }

      let firstRight = true;
      for (const item of controls) {
        const host = hosts.current.get(item.id);
        if (!host) continue;
        const hide = hidden.has(item.id);
        if (hide && host.contains(document.activeElement)) {
          root.current
            ?.querySelector<HTMLButtonElement>("[data-composer-expansion]")
            ?.focus({ preventScroll: true });
        }
        host.dataset.hidden = String(hide);
        host.dataset.menuOpen = String(openHosts.has(item.id));
        host.dataset.pinned = String(item.pinned === true);
        host.dataset.compactContext = String(
          item.id === "context" && item.pinned === true && required > capacity,
        );
        host.dataset.firstRight = String(
          !hide && item.side === "right" && firstRight,
        );
        host.dataset.measuring = "false";
        if (!hide && item.side === "right") firstRight = false;
      }
    }
  }, [items, menuCount, revision]);

  return (
    <div ref={root} className={styles.composer}>
      <div
        ref={top}
        className={styles.toolbar}
        data-testid="composer-top-toolbar"
      />
      {editor}
      <div
        ref={bottom}
        className={styles.toolbar}
        data-testid="composer-toolbar"
      />
      {items.map((item) => {
        const host = hosts.current.get(item.id);
        return host ? createPortal(item.content, host, item.id) : null;
      })}
    </div>
  );
}
