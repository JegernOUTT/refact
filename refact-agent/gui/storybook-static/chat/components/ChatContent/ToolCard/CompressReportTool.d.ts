import React from "react";
import { ToolCall } from "../../../services/refact/types";
interface CompressReportToolProps {
    toolCall: ToolCall;
    toolType: "compress_chat_probe" | "compress_chat_apply";
}
export declare const CompressReportTool: React.FC<CompressReportToolProps>;
export default CompressReportTool;
