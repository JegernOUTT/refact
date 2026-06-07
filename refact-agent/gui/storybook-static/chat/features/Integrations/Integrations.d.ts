import React from "react";
import type { Config } from "../Config/configSlice";
export type IntegrationsProps = {
    onCloseIntegrations?: () => void;
    backFromIntegrations: () => void;
    handlePaddingShift: (state: boolean) => void;
    host: Config["host"];
    tabbed: Config["tabbed"];
};
export type LeftRightPadding = {
    initial: string;
    xl: string;
    xs?: undefined;
    sm?: undefined;
    md?: undefined;
    lg?: undefined;
} | {
    initial: string;
    xs: string;
    sm: string;
    md: string;
    lg: string;
    xl: string;
};
export declare const Integrations: React.FC<IntegrationsProps>;
