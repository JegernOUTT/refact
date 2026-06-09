import React from "react";
import classNames from "classnames";
import styles from "./AnimatedList.module.css";

export type AnimatedListElement = "div" | "ul" | "tbody";

export interface AnimatedListProps extends React.HTMLAttributes<HTMLElement> {
  as?: AnimatedListElement;
  stagger?: boolean;
  initialItemLimit?: number;
}

function shouldAnimateChild(index: number, initialItemLimit: number) {
  return index < initialItemLimit;
}

export function AnimatedList({
  as,
  stagger = true,
  initialItemLimit = 12,
  className,
  children,
  ...props
}: AnimatedListProps) {
  const Component = as ?? "div";
  const safeLimit = Math.max(0, initialItemLimit);
  const enhancedChildren = React.Children.map(children, (child, index) => {
    if (!React.isValidElement<{ className?: string }>(child)) {
      return child;
    }

    if (!shouldAnimateChild(index, safeLimit)) {
      return child;
    }

    return React.cloneElement(child, {
      className: classNames(child.props.className, "rf-enter-rise"),
    });
  });

  return React.createElement(
    Component,
    {
      ...props,
      className: classNames(styles.list, stagger && "rf-stagger", className),
    },
    enhancedChildren,
  );
}
