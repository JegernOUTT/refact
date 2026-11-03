import React, { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  maxShrinkLevels,
  planComposerLayout,
  type ComposerLayoutItem,
  type ComposerRow,
} from "./composerLayout";
import styles from "./ComposerToolbar.module.css";

export type ComposerToolbarItem = {
  id: string;
  content: React.ReactNode;
  row: ComposerRow;
  side?: "left" | "right";
  pinned?: boolean;
  hidden?: boolean;
  squeezable?: boolean;
};

type Animatable = {
  animate?: (
    keyframes: Keyframe[],
    options: KeyframeAnimationOptions,
  ) => Animation;
  getAnimations?: () => Animation[];
};

const OPEN_TRIGGER =
  '[aria-haspopup][aria-expanded="true"], [aria-haspopup][data-state="open"]';
const SHRINK_LEVELS = maxShrinkLevels();
const ROWS: readonly ComposerRow[] = ["top", "bottom"];
const FALLBACK_MOVE_MS = 200;
const FALLBACK_ENTER_MS = 120;
const ENTER_SCALE = 0.92;
const MOVE_THRESHOLD_PX = 0.5;

function cssTime(element: Element, name: string, fallback: number): number {
  const raw = getComputedStyle(element).getPropertyValue(name).trim();
  const value = parseFloat(raw);
  if (Number.isNaN(value)) return fallback;
  return raw.endsWith("ms") ? value : value * 1000;
}

function cssValue(element: Element, name: string, fallback: string): string {
  return getComputedStyle(element).getPropertyValue(name).trim() || fallback;
}

function motionAllowed(): boolean {
  const media = (window as { matchMedia?: Window["matchMedia"] }).matchMedia;
  if (!media) return false;
  return !media("(prefers-reduced-motion: reduce)").matches;
}

function rowMetrics(element: HTMLElement | null) {
  if (!element) return { capacity: 0, gap: 0 };
  const style = getComputedStyle(element);
  return {
    capacity:
      element.clientWidth -
      (parseFloat(style.paddingLeft) || 0) -
      (parseFloat(style.paddingRight) || 0),
    gap: parseFloat(style.columnGap) || 0,
  };
}

export function ComposerToolbar({
  items,
  editor,
  menuCount,
  expanded,
}: {
  items: ComposerToolbarItem[];
  editor: React.ReactNode;
  menuCount: number;
  expanded: boolean;
}) {
  const root = useRef<HTMLDivElement>(null);
  const top = useRef<HTMLDivElement>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const hosts = useRef(new Map<string, HTMLSpanElement>());
  const track = useRef<(host: HTMLElement) => void>();
  const frame = useRef<number>();
  const widths = useRef(new WeakMap<Element, number>());
  const settled = useRef(false);
  const itemsRef = useRef(items);
  itemsRef.current = items;
  const [revision, setRevision] = useState(0);

  for (const item of items) {
    if (hosts.current.has(item.id)) continue;
    const host = document.createElement("span");
    host.className = styles.item;
    host.dataset.composerControl = item.id;
    hosts.current.set(item.id, host);
  }

  const layoutKey = items
    .map((item) =>
      [
        item.id,
        item.row,
        item.side ?? "left",
        item.hidden === true,
        item.pinned === true,
        item.squeezable === true,
      ].join(":"),
    )
    .join("|");

  useLayoutEffect(() => {
    const observed = new WeakSet<Element>();
    const schedule = () => {
      if (frame.current !== undefined) return;
      frame.current = requestAnimationFrame(() => {
        frame.current = undefined;
        setRevision((value) => value + 1);
      });
    };
    const resize = new ResizeObserver((entries) => {
      let widthChanged = false;
      for (const entry of entries) {
        const width = entry.contentRect.width;
        if (widths.current.get(entry.target) === width) continue;
        widths.current.set(entry.target, width);
        widthChanged = true;
      }
      if (widthChanged) schedule();
    });
    const mutations = new MutationObserver((records) => {
      const masked = records.some((record) => {
        const node = record.target;
        const element = node instanceof Element ? node : node.parentElement;
        const host = element?.closest<HTMLElement>("[data-composer-control]");
        return (
          host != null &&
          (host.dataset.level !== "0" || host.dataset.squeeze === "true")
        );
      });
      if (masked) schedule();
    });
    track.current = (host) => {
      if (observed.has(host)) return;
      observed.add(host);
      resize.observe(host);
      mutations.observe(host, {
        childList: true,
        characterData: true,
        subtree: true,
      });
    };
    if (root.current) resize.observe(root.current);
    hosts.current.forEach((host) => track.current?.(host));
    return () => {
      track.current = undefined;
      resize.disconnect();
      mutations.disconnect();
      if (frame.current !== undefined) cancelAnimationFrame(frame.current);
      frame.current = undefined;
    };
  }, []);

  useLayoutEffect(() => {
    const rootElement = root.current;
    if (!rootElement) return;
    const rowElements: Record<ComposerRow, HTMLDivElement | null> = {
      top: top.current,
      bottom: bottom.current,
    };
    const current = itemsRef.current;

    const before = new Map<string, DOMRect>();
    const beforeLevels = new Map<string, string | undefined>();
    hosts.current.forEach((host, id) => {
      const motion: Animatable = host;
      if (
        host.isConnected &&
        host.dataset.hidden !== "true" &&
        host.dataset.empty !== "true"
      ) {
        before.set(id, host.getBoundingClientRect());
        beforeLevels.set(id, host.dataset.level);
      }
      motion.getAnimations?.().forEach((animation) => animation.cancel());
    });

    const openIds = new Set(
      current
        .filter(
          (item) => hosts.current.get(item.id)?.querySelector(OPEN_TRIGGER),
        )
        .map((item) => item.id),
    );

    const frozenLevels = new Map<string, string>();
    for (const id of openIds) {
      frozenLevels.set(id, hosts.current.get(id)?.dataset.level ?? "0");
    }

    const focusedBeforeMove = document.activeElement;
    for (const row of ROWS) {
      const rowElement = rowElements[row];
      if (!rowElement) continue;
      const inRow = current.filter((item) => item.row === row);
      const ordered = [
        ...inRow.filter((item) => item.side !== "right"),
        ...inRow.filter((item) => item.side === "right"),
      ];
      ordered.forEach((item, index) => {
        const host = hosts.current.get(item.id);
        if (!host || rowElement.children.item(index) === host) return;
        rowElement.insertBefore(host, rowElement.children.item(index));
      });
    }
    if (
      focusedBeforeMove instanceof HTMLElement &&
      focusedBeforeMove.isConnected &&
      document.activeElement !== focusedBeforeMove
    ) {
      focusedBeforeMove.focus({ preventScroll: true });
    }

    for (const item of current) {
      const host = hosts.current.get(item.id);
      if (!host) continue;
      host.dataset.side = item.side ?? "left";
      host.dataset.hidden = String(
        item.hidden === true && !openIds.has(item.id),
      );
      host.dataset.level = frozenLevels.get(item.id) ?? "0";
      host.dataset.empty = "false";
      host.dataset.squeeze = "false";
      host.dataset.firstRight = "false";
      host.dataset.menuOpen = String(openIds.has(item.id));
    }

    const measured: ComposerLayoutItem[] = [];
    for (const item of current) {
      const host = hosts.current.get(item.id);
      if (!host || host.dataset.hidden === "true") continue;
      const width = host.getBoundingClientRect().width;
      if (width <= 0) {
        host.dataset.empty = "true";
        continue;
      }
      const widths = [width];
      const frozen = frozenLevels.get(item.id);
      const maxLevel =
        frozen === undefined ? SHRINK_LEVELS.get(item.id) ?? 0 : 0;
      for (let level = 1; level <= maxLevel; level += 1) {
        host.dataset.level = String(level);
        widths.push(host.getBoundingClientRect().width);
      }
      host.dataset.level = frozen ?? "0";
      measured.push({
        id: item.id,
        row: item.row,
        pinned: item.pinned === true || frozen !== undefined,
        widths,
      });
    }

    const plan = planComposerLayout(measured, {
      top: rowMetrics(rowElements.top),
      bottom: rowMetrics(rowElements.bottom),
    });

    for (const item of measured) {
      const host = hosts.current.get(item.id);
      if (!host) continue;
      host.dataset.level =
        frozenLevels.get(item.id) ?? String(plan.levels.get(item.id) ?? 0);
      if (plan.hidden.has(item.id)) host.dataset.hidden = "true";
    }
    for (const item of current) {
      if (item.squeezable !== true || !plan.overflowing.has(item.row)) continue;
      const host = hosts.current.get(item.id);
      if (host && host.dataset.hidden !== "true") host.dataset.squeeze = "true";
    }
    for (const row of ROWS) {
      const rowElement = rowElements[row];
      if (!rowElement) continue;
      const firstRight = Array.from(rowElement.children).find(
        (child): child is HTMLElement =>
          child instanceof HTMLElement &&
          child.dataset.side === "right" &&
          child.dataset.hidden !== "true" &&
          child.dataset.empty !== "true",
      );
      if (firstRight) firstRight.dataset.firstRight = "true";
    }

    const focused = document.activeElement;
    if (focused) {
      for (const host of hosts.current.values()) {
        if (host.dataset.hidden !== "true" || !host.contains(focused)) continue;
        rootElement.querySelector("textarea")?.focus({ preventScroll: true });
        break;
      }
    }

    const after = new Map<string, DOMRect>();
    hosts.current.forEach((host, id) => {
      const rect = host.getBoundingClientRect();
      after.set(id, rect);
      widths.current.set(host, rect.width);
      track.current?.(host);
    });

    if (settled.current && motionAllowed()) {
      const moveMs = cssTime(rootElement, "--rf-dur-med", FALLBACK_MOVE_MS);
      const enterMs = cssTime(rootElement, "--rf-dur-fast", FALLBACK_ENTER_MS);
      const moveEasing = cssValue(rootElement, "--rf-ease-standard", "ease");
      const enterEasing = cssValue(rootElement, "--rf-ease-entrance", "ease");
      hosts.current.forEach((host, id) => {
        if (
          !host.isConnected ||
          host.dataset.hidden === "true" ||
          host.dataset.empty === "true"
        ) {
          return;
        }
        const motion: Animatable = host;
        if (!motion.animate) return;
        const next = after.get(id);
        const previous = before.get(id);
        if (!next) return;
        if (!previous) {
          motion.animate(
            [
              { opacity: 0, transform: `scale(${ENTER_SCALE})` },
              { opacity: 1, transform: "none" },
            ],
            { duration: enterMs, easing: enterEasing },
          );
          return;
        }
        const dx = previous.left - next.left;
        const dy = previous.top - next.top;
        const moved =
          Math.abs(dx) >= MOVE_THRESHOLD_PX ||
          Math.abs(dy) >= MOVE_THRESHOLD_PX;
        const relabeled = beforeLevels.get(id) !== host.dataset.level;
        if (!moved && !relabeled) return;
        motion.animate(
          [
            {
              opacity: relabeled ? 0 : 1,
              transform: moved ? `translate(${dx}px, ${dy}px)` : "none",
            },
            { opacity: 1, transform: "none" },
          ],
          { duration: moveMs, easing: moveEasing },
        );
      });
    }
    settled.current = true;
  }, [layoutKey, menuCount, revision, expanded]);

  return (
    <div ref={root} className={styles.composer} data-expanded={expanded}>
      <div className={styles.topReveal} data-open={expanded}>
        <div className={styles.topClip}>
          <div
            ref={top}
            className={styles.toolbar}
            data-row="top"
            data-testid="composer-top-toolbar"
          />
        </div>
      </div>
      {editor}
      <div
        ref={bottom}
        className={styles.toolbar}
        data-row="bottom"
        data-testid="composer-toolbar"
      />
      {items.map((item) => {
        const host = hosts.current.get(item.id);
        return host ? createPortal(item.content, host, item.id) : null;
      })}
    </div>
  );
}
