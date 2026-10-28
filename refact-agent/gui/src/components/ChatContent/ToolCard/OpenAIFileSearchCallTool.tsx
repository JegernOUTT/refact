import { FileText } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool } from "./OpenAIServerTool";
import { FileResults } from "./OpenAISearchResults";
import { recordList } from "./openaiServerToolData";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

const FILE_RESULT_LIMIT = 200;

export const OpenAIFileSearchCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const query = typeof args.query === "string" ? args.query : undefined;
  const results = recordList(args.results, FILE_RESULT_LIMIT);
  return (
    <OpenAIServerTool
      icon={<FileText />}
      meta={results.length > 0 ? `${results.length} matches` : undefined}
      output={<FileResults results={results} />}
      target={query}
      toolCall={toolCall}
      verb="File search"
    />
  );
};
