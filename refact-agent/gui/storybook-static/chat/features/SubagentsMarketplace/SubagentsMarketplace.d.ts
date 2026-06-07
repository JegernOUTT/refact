import React from "react";
import type { Config } from "../Config/configSlice";
type SubagentsMarketplaceProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    backFromMarketplace: () => void;
};
export declare const SubagentsMarketplace: React.FC<SubagentsMarketplaceProps>;
export {};
