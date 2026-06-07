import React from "react";
import type { ToolCall } from "../../services/refact/types";
type ToolType = "doc_list" | "doc_get";
type Props = {
    toolType: ToolType;
    content: string;
};
type TaskDocumentsToolProps = {
    toolCall: ToolCall;
    toolType: ToolType;
};
export declare const TaskDocumentsContent: React.FC<Props>;
export declare const TaskDocumentsView: React.FC<TaskDocumentsToolProps>;
export default TaskDocumentsView;
