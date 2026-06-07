import React from "react";
import { ChatContextFile, ToolCall } from "../../../services/refact/types";
type WebToolType = "web" | "web_search";
interface WebToolProps {
    toolCall: ToolCall;
    toolType: WebToolType;
    contextFiles?: ChatContextFile[];
}
export declare const WebTool: React.FC<WebToolProps>;
export default WebTool;
