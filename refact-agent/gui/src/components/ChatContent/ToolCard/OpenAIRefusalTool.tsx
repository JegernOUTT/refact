import { TriangleAlert } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool, ServerToolText } from "./OpenAIServerTool";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIRefusalTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const refusal =
    typeof args.refusal === "string"
      ? args.refusal
      : typeof args.text === "string"
        ? args.text
        : undefined;
  return (
    <OpenAIServerTool
      icon={<TriangleAlert />}
      output={refusal ? <ServerToolText text={refusal} /> : null}
      target={refusal}
      toolCall={toolCall}
      verb="Refusal"
    />
  );
};
