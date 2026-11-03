import React from "react";
import { ComboboxPopover, type ComboboxStore } from "@ariakit/react";
import classNames from "classnames";
import { type AnchorRect } from "./utils";
import { ScrollArea } from "../ScrollArea";
import { KitHeader } from "../ComposerKit";
import styles from "./ComboBox.module.css";

export const Popover: React.FC<
  React.PropsWithChildren & {
    store: ComboboxStore;
    hidden: boolean;
    getAnchorRect: (anchor: HTMLElement | null) => AnchorRect | null;
    maxWidth?: number | null;
    title: string;
    meta?: string;
  }
> = ({ maxWidth, children, title, meta, ...props }) => {
  const style = maxWidth
    ? ({ "--rf-combobox-anchor-width": `${maxWidth}px` } as React.CSSProperties)
    : undefined;

  return (
    <ComboboxPopover
      unmountOnHide
      fitViewport
      {...props}
      aria-label={title}
      className={classNames("rf-popover-motion", styles.popover)}
      style={style}
    >
      <div aria-hidden="true">
        <KitHeader title={title} meta={meta} />
      </div>
      <ScrollArea scrollbars="vertical" className={styles.scroll}>
        <div className={styles.list}>{children}</div>
      </ScrollArea>
    </ComboboxPopover>
  );
};
