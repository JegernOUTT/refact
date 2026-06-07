import React from "react";
import type { Config } from "../Config/configSlice";
type CommandsMarketplaceProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    backFromMarketplace: () => void;
};
export declare const CommandsMarketplace: React.FC<CommandsMarketplaceProps>;
export {};
