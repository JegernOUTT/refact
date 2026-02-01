import React from "react";
import { MagnifyingGlassIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { StreamingToolCard } from "./StreamingToolCard";

interface CodeReviewToolProps {
  toolCall: ToolCall;
}

export const CodeReviewTool: React.FC<CodeReviewToolProps> = ({ toolCall }) => {
  return (
    <StreamingToolCard
      toolCall={toolCall}
      icon={<MagnifyingGlassIcon />}
      summary="Review code"
    />
  );
};

export default CodeReviewTool;
