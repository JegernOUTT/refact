import { Image, Monitor } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool } from "./OpenAIServerTool";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIComputerCallOutputTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const output = parseToolArgs(parseToolArgs(state.parsedArgs).output);
  const images = typeof output.image_url === "string" ? [output.image_url] : [];
  return (
    <OpenAIServerTool
      icon={images.length > 0 ? <Image /> : <Monitor />}
      images={images}
      meta={images.length > 0 ? "screenshot" : undefined}
      target="Output"
      toolCall={toolCall}
      verb="Computer"
    />
  );
};
