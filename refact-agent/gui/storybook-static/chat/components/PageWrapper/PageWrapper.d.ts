import React from "react";
import type { Config } from "../../features/Config/configSlice";
type PageWrapperProps = {
    children: React.ReactNode;
    host: Config["host"];
    className?: string;
    style?: React.CSSProperties;
    noPadding?: boolean;
};
export declare const PageWrapper: React.FC<PageWrapperProps>;
export {};
