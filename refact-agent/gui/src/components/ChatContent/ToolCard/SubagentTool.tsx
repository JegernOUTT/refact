import { User } from "lucide-react";
import React, { useMemo } from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard, type ReportData } from "./ReportToolCard";
import {
  readCompactRecord,
  readCompactText,
} from "../../Tools/AgenticTool/compactPresentation";

interface SubagentToolProps {
  toolCall: ToolCall;
  presentation?: "panel" | "inline";
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
    summary:
      readCompactText({ task: taskMatch?.[1] }, "task") ??
      "Background work report",
    markdown: response || content,
  };
}

export const SubagentTool: React.FC<SubagentToolProps> = ({
  toolCall,
  presentation = "panel",
}) => {
  const args = useMemo(
    () => readCompactRecord(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const task = readCompactText(args, "task");
  const tools = readCompactText(args, "tools");
  const maxSteps = readCompactText(args, "max_steps");
  const modelType = readCompactText(args, "model_type");
  const modelName = readCompactText(args, "model_name");

  const summary =
    presentation === "inline"
      ? task ?? "Background work"
      : `Analyze "${task ?? "task"}"`;

  const meta =
    [
      tools && `tools: ${tools}`,
      maxSteps && `max: ${maxSteps}`,
      modelType && `model type: ${modelType}`,
      modelName && `model: ${modelName}`,
      args?.worktree && "worktree",
      args?.auto_merge === true && "auto merge",
      Array.isArray(args?.target_files) &&
        args.target_files.length > 0 &&
        `files: ${args.target_files.length}`,
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
      compact={presentation === "inline"}
      unboundedContent
    />
  );
};

export default SubagentTool;
