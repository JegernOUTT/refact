import React from "react";
import * as TabsPrimitive from "@radix-ui/react-tabs";
import classNames from "classnames";

import styles from "./Tabs.module.css";

export interface TabsProps extends TabsPrimitive.TabsProps {
  children?: React.ReactNode;
}
export interface TabsListProps extends TabsPrimitive.TabsListProps {
  activeIndex?: number;
  itemCount?: number;
}
export type TabsTriggerProps = TabsPrimitive.TabsTriggerProps;
export type TabsContentProps = TabsPrimitive.TabsContentProps;

interface IndicatorRect {
  left: number;
  width: number;
}

function TabsRoot({ children, ...props }: TabsProps) {
  return <TabsPrimitive.Root {...props}>{children}</TabsPrimitive.Root>;
}

const TabsList = React.forwardRef<HTMLDivElement, TabsListProps>(
  (
    { activeIndex = 0, children, className, itemCount, style, ...props },
    ref,
  ) => {
    const rawCount = itemCount ?? React.Children.count(children);
    const itemTotal = Number.isFinite(rawCount) ? Math.max(0, rawCount) : 0;
    const safeCount = Math.max(1, itemTotal);
    const rawActiveIndex = Number.isFinite(activeIndex) ? activeIndex : 0;
    const safeActiveIndex = Math.min(
      Math.max(0, rawActiveIndex),
      safeCount - 1,
    );

    const listRef = React.useRef<HTMLDivElement | null>(null);
    const activeTriggerRef = React.useRef<HTMLElement | null>(null);
    const [indicator, setIndicator] = React.useState<IndicatorRect | null>(
      null,
    );

    const attachList = React.useCallback(
      (node: HTMLDivElement | null) => {
        listRef.current = node;
        if (typeof ref === "function") {
          ref(node);
        } else if (ref) {
          ref.current = node;
        }
      },
      [ref],
    );

    React.useLayoutEffect(() => {
      const list = listRef.current;
      if (!list) return;

      const sync = () => {
        const active = list.querySelector<HTMLElement>('[data-state="active"]');
        const previousActive = activeTriggerRef.current;
        if (active !== previousActive) {
          activeTriggerRef.current = active;
        }

        if (active && typeof active.scrollIntoView === "function") {
          if (!previousActive || !list.contains(document.activeElement)) {
            const left = active.offsetLeft;
            const right = left + active.offsetWidth;
            const visibleLeft = list.scrollLeft;
            const visibleRight = visibleLeft + list.clientWidth;

            if (left < visibleLeft) list.scrollLeft = left;
            else if (right > visibleRight)
              list.scrollLeft = right - list.clientWidth;
          }
        }

        if (
          active &&
          previousActive &&
          active !== previousActive &&
          typeof active.scrollIntoView === "function" &&
          list.contains(document.activeElement)
        ) {
          active.scrollIntoView({ block: "nearest", inline: "nearest" });
        }

        const next =
          active && active.offsetWidth > 0
            ? { left: active.offsetLeft, width: active.offsetWidth }
            : null;

        setIndicator((prev) =>
          prev?.left === next?.left && prev?.width === next?.width
            ? prev
            : next,
        );
      };

      sync();

      const resizeObserver = new ResizeObserver(sync);
      resizeObserver.observe(list);
      list
        .querySelectorAll('[role="tab"]')
        .forEach((tab) => resizeObserver.observe(tab));

      const mutationObserver = new MutationObserver(sync);
      mutationObserver.observe(list, {
        attributeFilter: ["data-state"],
        subtree: true,
      });

      return () => {
        resizeObserver.disconnect();
        mutationObserver.disconnect();
      };
    }, [itemTotal, safeActiveIndex]);

    return (
      <TabsPrimitive.List
        {...props}
        ref={attachList}
        className={classNames(styles.list, className)}
        data-tabs-measured={indicator ? "true" : undefined}
        style={
          {
            ...style,
            "--rf-tabs-count": safeCount,
            "--rf-tabs-index": safeActiveIndex,
            ...(indicator && {
              "--rf-tabs-indicator-left": `${indicator.left}px`,
              "--rf-tabs-indicator-width": `${indicator.width}px`,
            }),
          } as React.CSSProperties
        }
      >
        {itemTotal > 0 ? (
          <span aria-hidden="true" className={styles.indicator} />
        ) : null}
        {children}
      </TabsPrimitive.List>
    );
  },
);
TabsList.displayName = "Tabs.List";

const TabsTrigger = React.forwardRef<HTMLButtonElement, TabsTriggerProps>(
  ({ children, className, ...props }, ref) => (
    <TabsPrimitive.Trigger
      {...props}
      ref={ref}
      className={classNames(styles.trigger, "rf-pressable", className)}
    >
      {children}
    </TabsPrimitive.Trigger>
  ),
);
TabsTrigger.displayName = "Tabs.Trigger";

const TabsContent = React.forwardRef<HTMLDivElement, TabsContentProps>(
  ({ className, ...props }, ref) => (
    <TabsPrimitive.Content
      {...props}
      ref={ref}
      className={classNames(styles.content, className)}
    />
  ),
);
TabsContent.displayName = "Tabs.Content";

export const Tabs = Object.assign(TabsRoot, {
  Content: TabsContent,
  List: TabsList,
  Trigger: TabsTrigger,
});
