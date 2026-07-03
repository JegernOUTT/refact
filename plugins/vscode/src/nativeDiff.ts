/* eslint-disable @typescript-eslint/naming-convention */
import * as vscode from "vscode";

export const REFACT_DIFF_SCHEME = "refact-diff";

const originalContentByKey = new Map<string, string>();
let diffKeyCounter = 0;

class RefactDiffContentProvider implements vscode.TextDocumentContentProvider {
    provideTextDocumentContent(uri: vscode.Uri): string {
        return originalContentByKey.get(uri.query) ?? "";
    }
}

export function registerRefactDiffContentProvider(context: vscode.ExtensionContext): void {
    const provider = new RefactDiffContentProvider();
    context.subscriptions.push(
        vscode.workspace.registerTextDocumentContentProvider(REFACT_DIFF_SCHEME, provider),
    );
}

export function buildRefactDiffBeforeUri(fileName: string, originalContent: string): vscode.Uri {
    const safeName = fileName
        .replace(/%/g, "%25")
        .replace(/#/g, "%23")
        .replace(/\?/g, "%3F");
    const key = `${Date.now()}-${diffKeyCounter++}`;
    originalContentByKey.set(key, originalContent);
    return vscode.Uri.parse(`${REFACT_DIFF_SCHEME}:${safeName}`).with({ query: key });
}

export function disposeRefactDiffBeforeUri(uri: vscode.Uri): void {
    originalContentByKey.delete(uri.query);
}

export async function closeRefactDiffTabsForUri(uri: vscode.Uri): Promise<void> {
    for (const group of vscode.window.tabGroups.all) {
        for (const tab of group.tabs) {
            const input = tab.input;
            if (
                input instanceof vscode.TabInputTextDiff &&
                input.original.scheme === REFACT_DIFF_SCHEME &&
                input.modified.fsPath === uri.fsPath &&
                !tab.isDirty
            ) {
                try {
                    await vscode.window.tabGroups.close(tab);
                } catch (error) {
                    console.warn("Failed to close refact diff tab", error);
                }
            }
        }
    }
}
