import React from "react";
import type { Config } from "../Config/configSlice";
type SkillsMarketplaceProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    backFromMarketplace: () => void;
};
export declare const SkillsMarketplace: React.FC<SkillsMarketplaceProps>;
export {};
