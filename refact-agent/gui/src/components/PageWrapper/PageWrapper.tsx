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
};

export const PageWrapper: React.FC<PageWrapperProps> = ({
  children,
  className,
  host,
  style,
  noPadding,
}) => {
  return (
    <div
      className={classNames(
        styles.PageWrapper,
        host === "web" ? styles.web : styles.ide,
        noPadding && styles.noPadding,
        className,
      )}
      style={style}
    >
      {children}
    </div>
  );
};
