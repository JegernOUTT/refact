import { Search } from "lucide-react";
import React from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";

interface CodeReviewToolProps {
  toolCall: ToolCall;
}

export const CodeReviewTool: React.FC<CodeReviewToolProps> = ({ toolCall }) => {
  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={<Search />}
      defaultSummary="Review code"
      unboundedContent
    />
  );
};

export default CodeReviewTool;
