import React from "react";
import { type ProjectLabelInfo } from "../../../utils/createProjectLabelsWithConflictMarkers";
export type IntegrationPathFieldProps = {
    configPath: string;
    projectPath: string;
    projectLabels: ProjectLabelInfo[];
    shouldBeFormatted: boolean;
    globalLabel?: string;
};
export declare const IntegrationPathField: React.FC<IntegrationPathFieldProps>;
