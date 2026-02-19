import React, { useMemo } from "react";
import { PersonIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { StreamingToolCard } from "./StreamingToolCard";

interface SubagentArgs {
  task?: string;
  expected_result?: string;
  tools?: string;
  max_steps?: string;
}

interface SubagentToolProps {
  toolCall: ToolCall;
}

export const SubagentTool: React.FC<SubagentToolProps> = ({ toolCall }) => {
  const args = useMemo<SubagentArgs>(() => {
    try {
      return JSON.parse(toolCall.function.arguments) as SubagentArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const summary = `Analyze "${args.task ?? "task"}"`;

  const meta =
    [
      args.tools && `tools: ${args.tools}`,
      args.max_steps && `max: ${args.max_steps}`,
    ]
      .filter(Boolean)
      .join(" · ") || null;

  return (
    <StreamingToolCard
      toolCall={toolCall}
      icon={<PersonIcon />}
      summary={summary}
      meta={meta}
    />
  );
};

export default SubagentTool;
