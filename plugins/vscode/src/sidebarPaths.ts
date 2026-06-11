import * as path from "path";

export type CurrentProjectInfoPayload = {
    name: string;
    workspaceRoots?: string[];
};

export function normalizeWindowsExtendedPath(fileName: string): string {
    const uncPrefix = "\\\\?\\UNC\\";
    const localPrefix = "\\\\?\\";

    if (fileName.startsWith(uncPrefix)) {
        return "\\\\" + fileName.slice(uncPrefix.length);
    }

    if (fileName.startsWith(localPrefix)) {
        return fileName.slice(localPrefix.length);
    }

    return fileName;
}

function isPathInsideRoot(candidate: string, root: string): boolean {
    const relativePath = path.relative(root, candidate);
    return relativePath === "" || (!relativePath.startsWith("..") && !path.isAbsolute(relativePath));
}

export function resolveFilePathWithinWorkspace(fileName: string, workspaceRoots: string[], activeFilePath?: string): string | undefined {
    const roots = workspaceRoots
        .filter(root => root.trim().length > 0)
        .map(root => path.resolve(root));

    if (roots.length === 0) {
        return undefined;
    }

    const formattedFileName = normalizeWindowsExtendedPath(fileName);
    const activePath = activeFilePath ? path.resolve(activeFilePath) : undefined;
    const activeRoot = activePath ? roots.find(root => isPathInsideRoot(activePath, root)) : undefined;
    const baseRoot = activeRoot ?? roots[0];
    const candidate = path.resolve(path.isAbsolute(formattedFileName) ? formattedFileName : path.join(baseRoot, formattedFileName));

    return roots.some(root => isPathInsideRoot(candidate, root)) ? candidate : undefined;
}

export function createCurrentProjectInfo(name: string, workspaceRoots: string[]): CurrentProjectInfoPayload {
    return workspaceRoots.length > 0 ? { name, workspaceRoots } : { name };
}
