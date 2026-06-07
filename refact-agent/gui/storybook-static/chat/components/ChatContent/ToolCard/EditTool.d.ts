import React from "react";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
interface EditToolProps {
    toolCall: ToolCall;
    diffs?: DiffChunk[];
    isActiveTool?: boolean;
}
export declare const EditTool: React.FC<EditToolProps>;
export default EditTool;
