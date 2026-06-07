import React from "react";
import { ComboboxPopover, type ComboboxStore } from "@ariakit/react";
import { Box } from "@radix-ui/themes";
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
    <Box
      asChild
      className={classNames(
        "rt-PopperContent",
        "rt-HoverCardContent",
        "rf-popover-motion",
        styles.popover,
      )}
      style={style}
    >
      <ComboboxPopover unmountOnHide fitViewport {...props}>
        <ScrollArea scrollbars="vertical" className={styles.popover__scroll}>
          <Box className={styles.popover__box}>{children}</Box>
        </ScrollArea>
      </ComboboxPopover>
    </Box>
  );
};
