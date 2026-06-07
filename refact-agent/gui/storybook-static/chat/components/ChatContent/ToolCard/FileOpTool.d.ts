import React from "react";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
type FileOpType = "mv" | "rm" | "add_workspace_folder";
interface FileOpToolProps {
    toolCall: ToolCall;
    toolType: FileOpType;
    diffs?: DiffChunk[];
    isActiveTool?: boolean;
}
export declare const FileOpTool: React.FC<FileOpToolProps>;
export default FileOpTool;
