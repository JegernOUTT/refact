import { User } from "lucide-react";
import React, { useMemo } from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard, type ReportData } from "./ReportToolCard";

interface SubagentArgs {
  task?: string;
  expected_result?: string;
  tools?: string;
  max_steps?: string;
  model_type?: string;
  model_name?: string;
  goal?: string;
  plan?: string;
  worktree?: string | boolean;
  auto_merge?: boolean;
  target_files?: string[];
}

interface SubagentToolProps {
  toolCall: ToolCall;
}

function extractSubagentReport(content: string): ReportData | null {
  const responseMarker = "## Response";
  const responseIndex = content.indexOf(responseMarker);
  if (!content.startsWith("# Subagent Result") || responseIndex === -1) {
    return null;
  }

  const taskMatch = content.match(/\*\*Task:\*\*\s*([^\n]+)/);
  const responseStart = responseIndex + responseMarker.length;
  const response = content.slice(responseStart).trim();

  return {
    summary: taskMatch ? `Subagent: ${taskMatch[1].trim()}` : "Subagent report",
    markdown: response || content,
  };
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
      args.model_type && `model type: ${args.model_type}`,
      args.model_name && `model: ${args.model_name}`,
      args.worktree && "worktree",
      args.auto_merge && "auto merge",
      args.target_files?.length && `files: ${args.target_files.length}`,
    ]
      .filter(Boolean)
      .join(" · ") || null;

  return (
    <ReportToolCard
      toolCall={toolCall}
      icon={<User />}
      defaultSummary={summary}
      meta={meta}
      extractReport={extractSubagentReport}
      unboundedContent
    />
  );
};

export default SubagentTool;
