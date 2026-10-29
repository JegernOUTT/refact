import React from "react";
import classNames from "classnames";
import { Surface } from "../../../components/ui";
import styles from "./BuddySurface.module.css";

export type BuddySurfaceLevel = "section" | "inset";

type BuddySurfaceElement = "div" | "section" | "article" | "aside" | "li";

export interface BuddySurfaceProps extends React.HTMLAttributes<HTMLElement> {
  level?: BuddySurfaceLevel;
  as?: BuddySurfaceElement;
  compact?: boolean;
  testId?: string;
}

export const BuddySurface = React.forwardRef<HTMLElement, BuddySurfaceProps>(
  function BuddySurface(
    {
      level = "section",
      as = "div",
      compact = false,
      testId,
      className,
      children,
      ...props
    },
    ref,
  ) {
    return (
      <Surface
        ref={ref}
        as={as}
        variant={level === "section" ? "glass" : "surface-1"}
        radius={level === "section" ? "card" : "control"}
        className={classNames(
          styles.surface,
          styles[level],
          compact && styles.compact,
          className,
        )}
        data-buddy-surface={level}
        data-testid={testId}
        {...props}
      >
        {children}
      </Surface>
    );
  },
);
