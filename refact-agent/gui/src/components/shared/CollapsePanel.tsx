import React from "react";
import styles from "./CollapsePanel.module.css";

type Props = {
  collapsed: boolean;
  children: React.ReactNode;
  className?: string;
};

export const CollapsePanel: React.FC<Props> = ({ collapsed, children, className }) => (
  <div
    className={`${styles.panel}${className ? ` ${className}` : ""}`}
    data-collapsed={collapsed || undefined}
  >
    <div className={styles.inner}>
      {children}
    </div>
  </div>
);
