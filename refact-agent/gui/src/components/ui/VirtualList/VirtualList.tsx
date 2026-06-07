import classNames from "classnames";
import React from "react";
import { Virtuoso } from "react-virtuoso";
import type { Components } from "react-virtuoso";

import styles from "./VirtualList.module.css";

export interface VirtualListProps<T>
  extends Omit<React.ComponentProps<"div">, "children" | "itemData"> {
  items: T[];
  renderItem: (item: T, index: number) => React.ReactNode;
  getItemKey?: (item: T, index: number) => React.Key;
  header?: React.ReactNode;
  footer?: React.ReactNode;
  emptyMessage?: React.ReactNode;
  height?: string | number;
}

export function VirtualList<T>({
  className,
  emptyMessage = "No items yet",
  footer,
  getItemKey,
  header,
  height = 360,
  items,
  renderItem,
  ...props
}: VirtualListProps<T>) {
  const components: Components<T> = {
    Footer: footer ? () => <div className={styles.footer}>{footer}</div> : undefined,
    Header: header ? () => <div className={styles.header}>{header}</div> : undefined,
  };

  return (
    <div {...props} className={classNames(styles.root, className)} style={{ height }}>
      {items.length ? (
        <Virtuoso
          className={styles.virtuoso}
          components={components}
          data={items}
          itemContent={(index, item) => <div className={styles.item}>{renderItem(item, index)}</div>}
          computeItemKey={getItemKey ? (index, item) => getItemKey(item, index) : undefined}
        />
      ) : (
        <div className={styles.empty}>{emptyMessage}</div>
      )}
    </div>
  );
}
