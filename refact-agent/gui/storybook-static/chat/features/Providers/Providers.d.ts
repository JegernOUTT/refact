import React from "react";
import type { Config } from "../Config/configSlice";
export type ProvidersProps = {
    backFromProviders: () => void;
    host: Config["host"];
    tabbed: Config["tabbed"];
};
export declare const Providers: React.FC<ProvidersProps>;
