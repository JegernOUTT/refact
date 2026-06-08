import React from "react";
import {
  Combobox as AriakitCombobox,
  ComboboxItem,
  ComboboxPopover,
  useComboboxStore,
} from "@ariakit/react";
import classNames from "classnames";
import { Check, ChevronDown } from "lucide-react";

import { Portal } from "../../Portal";
import { Icon } from "../Icon";
import { overlayStyle } from "../overlayTypes";
import type { OverlaySide } from "../overlayTypes";
import styles from "./Combobox.module.css";

export interface ComboboxItemOption {
  value: string;
  label?: React.ReactNode;
  disabled?: boolean;
}

export interface ComboboxProps
  extends Omit<
    React.ComponentPropsWithoutRef<"input">,
    "children" | "onChange" | "onSelect" | "value"
  > {
  items: ComboboxItemOption[];
  value: string;
  onValueChange: (value: string) => void;
  onSelect?: (item: ComboboxItemOption) => void;
  emptyLabel?: React.ReactNode;
  maxWidth?: string;
  maxHeight?: string;
  side?: OverlaySide;
  align?: "start" | "end";
}

export function Combobox({
  align = "start",
  className,
  emptyLabel = "No matches",
  items,
  maxHeight,
  maxWidth,
  onSelect,
  onValueChange,
  placeholder = "Search…",
  side = "bottom",
  value,
  ...props
}: ComboboxProps) {
  const store = useComboboxStore({
    defaultOpen: false,
    placement: `${side}-${align}`,
    value,
    setValue: onValueChange,
  });
  const open = store.useState("open");
  const matches = React.useMemo(() => {
    const normalizedValue = value.trim().toLocaleLowerCase();

    if (!normalizedValue) {
      return items;
    }

    return items.filter((item) =>
      item.value.toLocaleLowerCase().includes(normalizedValue),
    );
  }, [items, value]);

  React.useEffect(() => {
    store.setOpen(matches.length > 0 && value.length > 0);
  }, [matches.length, store, value.length]);

  return (
    <div className={styles.root}>
      <div className={styles.control} data-open={open ? "true" : undefined}>
        <AriakitCombobox
          {...props}
          store={store}
          className={classNames(styles.input, className)}
          placeholder={placeholder}
        />
        <Icon icon={ChevronDown} size="sm" tone="muted" />
      </div>
      <Portal>
        <ComboboxPopover
          store={store}
          className={styles.popover}
          gutter={8}
          sameWidth
          style={overlayStyle(maxWidth, maxHeight)}
        >
          {matches.length > 0 ? (
            matches.map((item) => (
              <ComboboxItem
                key={item.value}
                className={styles.item}
                data-selected={item.value === value ? "true" : undefined}
                disabled={item.disabled}
                focusOnHover
                value={item.value}
                onClick={() => {
                  onSelect?.(item);
                  store.hide();
                }}
              >
                <span className={styles.itemLabel}>
                  {item.label ?? item.value}
                </span>
                {item.value === value ? (
                  <Icon icon={Check} size="sm" tone="accent" />
                ) : null}
              </ComboboxItem>
            ))
          ) : (
            <div className={styles.empty}>{emptyLabel}</div>
          )}
        </ComboboxPopover>
      </Portal>
    </div>
  );
}
