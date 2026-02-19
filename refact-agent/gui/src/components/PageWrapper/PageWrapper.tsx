import React, { useMemo } from "react";
import { Flex } from "@radix-ui/themes";
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
  const xPadding = useMemo(() => {
    if (host === "web") return { initial: "4", xl: "6" };
    return {
      initial: "2",
      xs: "2",
      sm: "3",
      md: "4",
      lg: "5",
      xl: "6",
    };
  }, [host]);

  const yPadding = useMemo(() => {
    return host === "web" ? "5" : "2";
  }, [host]);

  return (
    <Flex
      direction="column"
      justify="between"
      flexGrow="1"
      py={noPadding ? "0" : yPadding}
      px={noPadding ? "2" : xPadding}
      className={classNames(styles.PageWrapper, className)}
      style={style}
    >
      {children}
    </Flex>
  );
};
