import { Monitor } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool } from "./OpenAIServerTool";
import { humanizeToolName, parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIComputerCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const action = typeof args.action === "string" ? args.action : undefined;
  return (
    <OpenAIServerTool
      icon={<Monitor />}
      target={action ? humanizeToolName(action) : undefined}
      toolCall={toolCall}
      verb="Computer"
    />
  );
};
