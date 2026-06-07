import React from "react";
import type { ExecProcessStatus } from "../../../services/refact/types";
type ProcessStatusValue = ExecProcessStatus | (string & Record<never, never>);
type ProcessStatusBadgeProps = {
    status: ProcessStatusValue;
};
export declare const ProcessStatusBadge: React.FC<ProcessStatusBadgeProps>;
export default ProcessStatusBadge;
