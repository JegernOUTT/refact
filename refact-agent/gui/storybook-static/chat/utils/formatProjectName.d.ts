/**
 * Formats the project path to display only the last folder.
 * @param projectPath - The full path of the project.
 * @param isMarkdown (optional) - Rather project name should be formatted to be inserted in markdown.
 * @param indexOfLastFolder (optional) - Indicates which folder to extract from the path. (from right to left)
 * @returns The formatted project name.
 */
export declare const formatProjectName: ({ projectPath, isMarkdown, indexOfLastFolder, }: {
    projectPath: string;
    isMarkdown?: boolean;
    indexOfLastFolder?: number;
}) => string;
