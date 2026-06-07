import React from "react";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
interface ListToolProps {
    toolCall: ToolCall;
    contextFiles?: ChatContextFile[];
}
export declare const ListTool: React.FC<ListToolProps>;
export default ListTool;
