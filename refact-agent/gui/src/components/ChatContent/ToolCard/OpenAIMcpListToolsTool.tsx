import { Box } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { OpenAIServerTool } from "./OpenAIServerTool";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIMcpListToolsTool: React.FC<Props> = ({ toolCall }) => (
  <OpenAIServerTool
    icon={<Box />}
    target="List tools"
    toolCall={toolCall}
    verb="MCP"
  />
);
