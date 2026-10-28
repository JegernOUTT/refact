import {
  Cuboid,
  FileText,
  Image,
  Monitor,
  Search,
  Timer,
  TriangleAlert,
  Volume2,
} from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool, ServerToolText } from "./OpenAIServerTool";
import { FileResults, WebResults } from "./OpenAISearchResults";
import { codeInterpreterOutputs, recordList } from "./openaiServerToolData";
import { parseToolArgs } from "./genericToolTitle";

type OpenAIResponsesToolProps = {
  toolCall: ToolCall;
};

const ICONS: Partial<Record<string, React.ReactNode>> = {
  openai_web_search_call: <Search />,
  openai_file_search_call: <FileText />,
  openai_code_interpreter_call: <Timer />,
  openai_computer_call: <Monitor />,
  openai_computer_call_output: <Monitor />,
  openai_audio: <Volume2 />,
  openai_image_generation_call: <Image />,
  openai_refusal: <TriangleAlert />,
};

function payloadOutput(
  toolName: string,
  args: Record<string, unknown>,
): React.ReactNode {
  if (toolName === "openai_web_search_call")
    return <WebResults results={recordList(args.results, 20)} />;
  if (toolName === "openai_file_search_call")
    return <FileResults results={recordList(args.results, 50)} />;
  if (toolName === "openai_code_interpreter_call") {
    const { text } = codeInterpreterOutputs(args.outputs);
    return text.length > 0 ? <ServerToolText text={text.join("\n\n")} /> : null;
  }
  return null;
}

export const OpenAIResponsesTool: React.FC<OpenAIResponsesToolProps> = ({
  toolCall,
}) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const query = typeof args.query === "string" ? args.query : undefined;
  const type = typeof args.type === "string" ? args.type : undefined;
  return (
    <OpenAIServerTool
      icon={ICONS[state.toolName] ?? <Cuboid />}
      meta={query ? undefined : type}
      output={payloadOutput(state.toolName, args)}
      target={query}
      toolCall={toolCall}
      verb={state.label}
    />
  );
};
