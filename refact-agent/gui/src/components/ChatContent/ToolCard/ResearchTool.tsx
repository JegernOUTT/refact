import React, { useMemo } from "react";
import { ReaderIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { StreamingToolCard } from "./StreamingToolCard";
import { truncateMiddle } from "./utils";

interface ResearchArgs {
  research_query?: string;
}

interface ResearchToolProps {
  toolCall: ToolCall;
}

export const ResearchTool: React.FC<ResearchToolProps> = ({ toolCall }) => {
  const args = useMemo<ResearchArgs>(() => {
    try {
      return JSON.parse(toolCall.function.arguments) as ResearchArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const query = args.research_query ?? "";
  const summary = query
    ? `Research "${truncateMiddle(query, 45)}"`
    : "Research";

  return (
    <StreamingToolCard
      toolCall={toolCall}
      icon={<ReaderIcon />}
      summary={summary}
    />
  );
};

export default ResearchTool;
