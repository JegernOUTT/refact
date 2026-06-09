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

function TabsRoot({ children, ...props }: TabsProps) {
  return <TabsPrimitive.Root {...props}>{children}</TabsPrimitive.Root>;
}

const TabsList = React.forwardRef<HTMLDivElement, TabsListProps>(
  ({ activeIndex = 0, children, className, itemCount, style, ...props }, ref) => {
    const rawCount = itemCount ?? React.Children.count(children);
    const itemTotal = Number.isFinite(rawCount) ? Math.max(0, rawCount) : 0;
    const safeCount = Math.max(1, itemTotal);
    const rawActiveIndex = Number.isFinite(activeIndex) ? activeIndex : 0;
    const safeActiveIndex = Math.min(Math.max(0, rawActiveIndex), safeCount - 1);

    return (
      <TabsPrimitive.List
        {...props}
        ref={ref}
        className={classNames(styles.list, className)}
        style={
          {
            ...style,
            "--rf-tabs-count": safeCount,
            "--rf-tabs-index": safeActiveIndex,
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
