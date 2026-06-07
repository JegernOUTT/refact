export type ProjectLabelInfo = {
    path: string;
    label: string;
    fullPath: string;
    hasConflict: boolean;
};
/**
 * Creates project labels and marks conflicting ones for tooltip display.
 * @param projectPaths - Array of project paths
 * @param indexOfLastFolder - Number of folders to show from the end (default: 1)
 * @returns Array of ProjectLabelInfo objects with conflict markers
 */
export declare const createProjectLabelsWithConflictMarkers: (projectPaths: string[], indexOfLastFolder?: number) => ProjectLabelInfo[];
