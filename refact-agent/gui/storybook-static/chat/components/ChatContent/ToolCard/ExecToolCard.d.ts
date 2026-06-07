import React from "react";
import type { ToolCall } from "../../../services/refact/types";
type ExecToolName = "shell" | "shell_service" | "process_start" | "process_list" | "process_read" | "process_kill" | "process_wait" | "process_write_stdin" | "exec";
type ExecToolCardProps = {
    toolCall: ToolCall;
    toolName: ExecToolName;
};
export declare const ExecToolCard: React.FC<ExecToolCardProps>;
export default ExecToolCard;
