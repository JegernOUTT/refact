import React from "react";
import styles from "./PageWrapper.module.css";
import classNames from "classnames";
import type { Config } from "../../features/Config/configSlice";

type PageWrapperProps = {
  children: React.ReactNode;
  host: Config["host"];
  className?: string;
  style?: React.CSSProperties;
  noPadding?: boolean;
  flush?: boolean;
};

export const PageWrapper: React.FC<PageWrapperProps> = ({
  children,
  className,
  host,
  style,
  noPadding,
  flush,
}) => {
  return (
    <div
      className={classNames(
        styles.PageWrapper,
        host === "web" ? styles.web : styles.ide,
        noPadding && styles.noPadding,
        flush && styles.flush,
        className,
      )}
      style={style}
    >
      {children}
    </div>
  );
};
