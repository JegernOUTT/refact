import React from "react";
import { ComboboxPopover, type ComboboxStore } from "@ariakit/react";
import classNames from "classnames";
import { type AnchorRect } from "./utils";
import { ScrollArea } from "../ScrollArea";
import styles from "./ComboBox.module.css";

export const Popover: React.FC<
  React.PropsWithChildren & {
    store: ComboboxStore;
    hidden: boolean;
    getAnchorRect: (anchor: HTMLElement | null) => AnchorRect | null;
    maxWidth?: number | null;
  }
> = ({ maxWidth, children, ...props }) => {
  const style = maxWidth
    ? ({ "--rf-combobox-anchor-width": `${maxWidth}px` } as React.CSSProperties)
    : undefined;

  return (
    <ComboboxPopover
      unmountOnHide
      fitViewport
      {...props}
      className={classNames("rf-popover-motion", styles.popover)}
      style={style}
    >
      <ScrollArea scrollbars="vertical" className={styles.popover__scroll}>
        <div className={styles.popover__box}>{children}</div>
      </ScrollArea>
    </ComboboxPopover>
  );
};
