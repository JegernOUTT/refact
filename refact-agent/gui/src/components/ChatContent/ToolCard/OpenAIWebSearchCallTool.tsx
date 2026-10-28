import { Search } from "lucide-react";
import React from "react";
import type { ToolCall } from "../../../services/refact/types";
import { useOpenAiResponsesToolCardState } from "./openaiResponsesToolCardState";
import { OpenAIServerTool } from "./OpenAIServerTool";
import { WebResults } from "./OpenAISearchResults";
import { recordList } from "./openaiServerToolData";
import { parseToolArgs } from "./genericToolTitle";

type Props = {
  toolCall: ToolCall;
};

const WEB_RESULT_LIMIT = 50;

export const OpenAIWebSearchCallTool: React.FC<Props> = ({ toolCall }) => {
  const state = useOpenAiResponsesToolCardState(toolCall);
  const args = parseToolArgs(state.parsedArgs);
  const query = typeof args.query === "string" ? args.query : undefined;
  const results = recordList(args.results, WEB_RESULT_LIMIT);
  return (
    <OpenAIServerTool
      icon={<Search />}
      meta={results.length > 0 ? `${results.length} results` : undefined}
      output={<WebResults results={results} />}
      target={query}
      toolCall={toolCall}
      verb="Web search"
    />
  );
};
