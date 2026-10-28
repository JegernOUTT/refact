import { Telescope } from "lucide-react";
import React, { useMemo } from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";
import { extractSubagentReport } from "./subagentReport";
import {
  readCompactRecord,
  readCompactText,
} from "../../Tools/AgenticTool/compactPresentation";
import { Markdown } from "../../Markdown";
import { TranscriptClamp, type TranscriptFact } from "../TranscriptItem";

interface SubagentToolProps {
  toolCall: ToolCall;
  presentation?: "panel" | "inline";
}

export const SubagentTool: React.FC<SubagentToolProps> = ({ toolCall }) => {
  const args = useMemo(
    () => readCompactRecord(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const task = typeof args?.task === "string" ? args.task.trim() : "";
  const model =
    readCompactText(args, "model_name") ?? readCompactText(args, "model_type");
  const maxSteps = readCompactText(args, "max_steps");
  const targetFiles = Array.isArray(args?.target_files)
    ? args.target_files.length
    : 0;

  const facts: TranscriptFact[] = [
    ...(model ? [{ label: "Model", value: model }] : []),
    ...(args?.worktree
      ? [
          {
            label: "Worktree",
            value:
              args.auto_merge === true ? "isolated · auto merge" : "isolated",
          },
        ]
      : []),
    ...(maxSteps ? [{ label: "Max steps", value: maxSteps }] : []),
    ...(targetFiles > 0
      ? [{ label: "Target files", value: String(targetFiles) }]
      : []),
  ];
  const tools = readCompactText(args, "tools");

  return (
    <ReportToolCard
      defaultSummary={task || "Background work"}
      extractReport={extractSubagentReport}
      facts={facts}
      icon={<Telescope />}
      input={
        task ? (
          <TranscriptClamp>
            <Markdown>{task}</Markdown>
          </TranscriptClamp>
        ) : null
      }
      meta={tools ? `tools: ${tools}` : null}
      toolCall={toolCall}
      verb="Subagent"
    />
  );
};

export default SubagentTool;
