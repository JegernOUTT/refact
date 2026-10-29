import React, { useCallback, useLayoutEffect, useRef, useState } from "react";
import classNames from "classnames";
import styles from "./BuddyScrollRail.module.css";

type Edges = { start: boolean; end: boolean };

const NO_EDGES: Edges = { start: false, end: false };
const SELECTED = 'input:checked, [aria-selected="true"], [aria-current]';

export interface BuddyScrollRailProps {
  children: React.ReactNode;
  watch?: unknown;
  className?: string;
  railRef?: React.Ref<HTMLDivElement>;
}

export function BuddyScrollRail({
  children,
  watch,
  className,
  railRef,
}: BuddyScrollRailProps) {
  const viewport = useRef<HTMLDivElement | null>(null);
  const [edges, setEdges] = useState<Edges>(NO_EDGES);

  const attach = useCallback(
    (node: HTMLDivElement | null) => {
      viewport.current = node;
      if (typeof railRef === "function") railRef(node);
      else if (railRef)
        (railRef as React.MutableRefObject<HTMLDivElement | null>).current =
          node;
    },
    [railRef],
  );

  const measure = useCallback(() => {
    const node = viewport.current;
    if (!node) return;
    const max = node.scrollWidth - node.clientWidth;
    const next = {
      start: max > 1 && node.scrollLeft > 1,
      end: max > 1 && node.scrollLeft < max - 1,
    };
    setEdges((prev) =>
      prev.start === next.start && prev.end === next.end ? prev : next,
    );
  }, []);

  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node) return;
    const selected = node
      .querySelector<HTMLElement>(SELECTED)
      ?.closest<HTMLElement>("label, [role='tab'], a, button");
    if (selected) {
      const box = node.getBoundingClientRect();
      const item = selected.getBoundingClientRect();
      if (item.left < box.left) node.scrollLeft -= box.left - item.left;
      else if (item.right > box.right)
        node.scrollLeft += item.right - box.right;
    }
    measure();
  }, [measure, watch]);

  useLayoutEffect(() => {
    const node = viewport.current;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    if (node.firstElementChild) observer.observe(node.firstElementChild);
    return () => observer.disconnect();
  }, [measure]);

  return (
    <div
      ref={attach}
      className={classNames(styles.rail, className)}
      data-overflow-start={edges.start || undefined}
      data-overflow-end={edges.end || undefined}
      onScroll={measure}
    >
      {children}
    </div>
  );
}
