import { Volume2 } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool, ServerToolText } from "./OpenAIServerTool";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIAudioTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const transcript =
    typeof args.transcript === "string"
      ? args.transcript
      : typeof args.text === "string"
        ? args.text
        : undefined;
  return (
    <OpenAIServerTool
      icon={<Volume2 />}
      output={transcript ? <ServerToolText text={transcript} /> : null}
      target={transcript}
      toolCall={toolCall}
      verb="Audio"
    />
  );
};
