import React from "react";
import type { Config } from "../Config/configSlice";
type MCPMarketplaceProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    backFromMarketplace: () => void;
};
export declare const MCPMarketplace: React.FC<MCPMarketplaceProps>;
export {};
