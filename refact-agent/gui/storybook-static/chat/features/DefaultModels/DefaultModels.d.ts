import React from "react";
import type { Config } from "../Config/configSlice";
type DefaultModelsProps = {
    backFromDefaultModels: () => void;
    host: Config["host"];
    tabbed: Config["tabbed"];
    draftId?: string;
};
export declare const DefaultModels: React.FC<DefaultModelsProps>;
export {};
