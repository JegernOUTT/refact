import React, { useMemo } from "react";
import { CodeIcon } from "@radix-ui/react-icons";
import { ToolCall } from "../../../services/refact/types";
import { StreamingToolCard } from "./StreamingToolCard";
import { truncateMiddle } from "./utils";

interface ShellArgs {
  command?: string;
  workdir?: string;
}

interface ShellToolProps {
  toolCall: ToolCall;
}

export const ShellTool: React.FC<ShellToolProps> = ({ toolCall }) => {
  const args = useMemo<ShellArgs>(() => {
    try {
      return JSON.parse(toolCall.function.arguments) as ShellArgs;
    } catch {
      return {};
    }
  }, [toolCall.function.arguments]);

  const command = args.command ?? toolCall.function.arguments;
  const summary = `Run ${truncateMiddle(command, 50)}`;
  const meta = args.workdir ? `in ${truncateMiddle(args.workdir, 40)}` : null;

  return (
    <StreamingToolCard
      toolCall={toolCall}
      icon={<CodeIcon />}
      summary={summary}
      meta={meta}
    />
  );
};

export default ShellTool;
