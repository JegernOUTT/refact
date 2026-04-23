import React, { useMemo } from "react";
import { ReaderIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";

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
  const summary = query ? `Research "${query}"` : "Research";

  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={<ReaderIcon />}
      defaultSummary={summary}
    />
  );
};

export default ResearchTool;
