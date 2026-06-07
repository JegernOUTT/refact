import React from "react";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
interface ReadToolProps {
    toolCall: ToolCall;
    contextFiles?: ChatContextFile[];
}
export declare const ReadTool: React.FC<ReadToolProps>;
export default ReadTool;
