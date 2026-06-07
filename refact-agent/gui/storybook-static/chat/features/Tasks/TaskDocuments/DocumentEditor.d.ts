import React from "react";
type DocumentEditorProps = {
    taskId: string;
    mode: "create" | "edit";
    slug?: string;
    open: boolean;
    onOpenChange: (open: boolean) => void;
};
export declare const DocumentEditor: React.FC<DocumentEditorProps>;
export default DocumentEditor;
