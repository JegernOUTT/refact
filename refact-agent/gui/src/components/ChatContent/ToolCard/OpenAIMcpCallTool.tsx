import { Box } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool } from "./OpenAIServerTool";
import { humanizeToolName, parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIMcpCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const server = typeof args.server === "string" ? args.server : undefined;
  const tool = typeof args.tool === "string" ? args.tool : undefined;
  return (
    <OpenAIServerTool
      icon={<Box />}
      meta={server}
      target={tool ? humanizeToolName(tool) : undefined}
      toolCall={toolCall}
      verb="MCP"
    />
  );
};
