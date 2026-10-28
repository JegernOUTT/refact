import { Timer } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool, ServerToolText } from "./OpenAIServerTool";
import { codeInterpreterOutputs } from "./openaiServerToolData";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

export const OpenAICodeInterpreterCallTool: React.FC<Props> = ({
  toolCall,
}) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const { text, images } = codeInterpreterOutputs(args.outputs);
  const code = typeof args.code === "string" ? args.code : undefined;
  const count = text.length + images.length;
  return (
    <OpenAIServerTool
      icon={<Timer />}
      images={images}
      meta={count > 0 ? `${count} outputs` : undefined}
      output={
        code !== undefined || text.length > 0 ? (
          <>
            {code ? <ServerToolText text={code} /> : null}
            {text.length > 0 ? (
              <ServerToolText text={text.join("\n\n")} />
            ) : null}
          </>
        ) : null
      }
      toolCall={toolCall}
      verb="Code interpreter"
    />
  );
};
