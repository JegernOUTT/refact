import React from "react";
import type { Config } from "../Config/configSlice";
type MarketplaceHubProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    back: () => void;
};
export declare const MarketplaceHub: React.FC<MarketplaceHubProps>;
export {};
