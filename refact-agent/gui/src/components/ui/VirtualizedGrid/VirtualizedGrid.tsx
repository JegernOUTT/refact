import React, { useLayoutEffect, useMemo, useRef, useState } from "react";
import classNames from "classnames";
import { Virtuoso } from "react-virtuoso";

import { getScrollParent } from "../../../utils/getScrollParent";
import styles from "./VirtualizedGrid.module.css";

export interface VirtualizedGridProps<T> {
  items: T[];
  renderItem: (item: T, index: number) => React.ReactNode;
  getItemKey?: (item: T, index: number) => React.Key;
  minColumnWidth?: number;
  columns?: number;
  gap?: number;
  rowHeight?: number;
  virtualizeThreshold?: number;
  className?: string;
  "aria-label"?: string;
}

const DEFAULT_MIN_COLUMN_WIDTH = 260;
const DEFAULT_GAP = 12;
const DEFAULT_VIRTUALIZE_THRESHOLD = 80;
const FALLBACK_VIRTUAL_HEIGHT = "70dvh";

function chunkRows<T>(items: T[], size: number): T[][] {
  if (size <= 1) return items.map((item) => [item]);
  const rows: T[][] = [];
  for (let index = 0; index < items.length; index += size) {
    rows.push(items.slice(index, index + size));
  }
  return rows;
}

export function VirtualizedGrid<T>({
  items,
  renderItem,
  getItemKey,
  minColumnWidth = DEFAULT_MIN_COLUMN_WIDTH,
  columns,
  gap = DEFAULT_GAP,
  rowHeight,
  virtualizeThreshold = DEFAULT_VIRTUALIZE_THRESHOLD,
  className,
  "aria-label": ariaLabel,
}: VirtualizedGridProps<T>) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [scrollParent, setScrollParent] = useState<HTMLElement | null>(null);
  const [scrollParentResolved, setScrollParentResolved] = useState(false);

  const shouldVirtualize = items.length > virtualizeThreshold;

  useLayoutEffect(() => {
    const element = wrapperRef.current;
    if (!element) return;
    const measure = () => setWidth(element.getBoundingClientRect().width);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (!shouldVirtualize) {
      setScrollParent(null);
      setScrollParentResolved(false);
      return;
    }
    setScrollParent(getScrollParent(wrapperRef.current));
    setScrollParentResolved(true);
  }, [shouldVirtualize]);

  const resolvedColumns = useMemo(() => {
    if (columns && columns > 0) return columns;
    if (width <= 0) return 1;
    return Math.max(1, Math.floor((width + gap) / (minColumnWidth + gap)));
  }, [columns, width, gap, minColumnWidth]);

  const tileStyle = useMemo<React.CSSProperties | undefined>(
    () => (rowHeight ? { height: rowHeight } : undefined),
    [rowHeight],
  );

  const renderTile = (item: T, index: number) => (
    <div
      key={getItemKey ? getItemKey(item, index) : index}
      className={styles.tile}
      style={tileStyle}
    >
      {renderItem(item, index)}
    </div>
  );

  if (items.length === 0) return null;

  if (!shouldVirtualize) {
    const templateColumns =
      columns && columns > 0
        ? `repeat(${columns}, minmax(0, 1fr))`
        : `repeat(auto-fill, minmax(min(100%, ${minColumnWidth}px), 1fr))`;
    return (
      <div
        ref={wrapperRef}
        aria-label={ariaLabel}
        className={classNames(styles.grid, "rf-stagger", className)}
        style={{ gridTemplateColumns: templateColumns, gap }}
      >
        {items.map((item, index) => renderTile(item, index))}
      </div>
    );
  }

  if (!scrollParentResolved) {
    return <div ref={wrapperRef} className={className} />;
  }

  const rows = chunkRows(items, resolvedColumns);
  const rowTemplate =
    resolvedColumns > 1
      ? `repeat(${resolvedColumns}, minmax(0, 1fr))`
      : "minmax(0, 1fr)";

  return (
    <div ref={wrapperRef} aria-label={ariaLabel} className={className}>
      <Virtuoso
        customScrollParent={scrollParent ?? undefined}
        data={rows}
        computeItemKey={(rowIndex) => `row-${rowIndex}-cols-${resolvedColumns}`}
        itemContent={(rowIndex, row) => (
          <div
            className={styles.row}
            style={{
              gridTemplateColumns: rowTemplate,
              gap,
              paddingBottom: gap,
            }}
          >
            {row.map((item, colIndex) =>
              renderTile(item, rowIndex * resolvedColumns + colIndex),
            )}
          </div>
        )}
        style={scrollParent ? undefined : { height: FALLBACK_VIRTUAL_HEIGHT }}
      />
    </div>
  );
}
