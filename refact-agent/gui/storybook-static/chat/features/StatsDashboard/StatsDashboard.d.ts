import React from "react";
import type { Config } from "../Config/configSlice";
export type StatsDashboardProps = {
    host: Config["host"];
    tabbed: Config["tabbed"];
    backFromDashboard: () => void;
};
export declare const StatsDashboard: React.FC<StatsDashboardProps>;
