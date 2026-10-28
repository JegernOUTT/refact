import { Image } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool } from "./OpenAIServerTool";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAIImageGenerationCallTool: React.FC<Props> = ({
  toolCall,
}) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const output = parseToolArgs(args.output);
  const images = [args.image_url, output.image_url].filter(
    (url): url is string => typeof url === "string" && url.length > 0,
  );
  const prompt =
    typeof args.revised_prompt === "string" ? args.revised_prompt : undefined;
  return (
    <OpenAIServerTool
      icon={<Image />}
      images={images}
      target={prompt}
      toolCall={toolCall}
      verb="Image generation"
    />
  );
};
