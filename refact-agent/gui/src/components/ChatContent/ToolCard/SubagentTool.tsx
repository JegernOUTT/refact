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
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useThreadId } from "../../../features/Chat/Thread";
import { selectFoldedAgentEvent } from "../foldedEventSelectors";
import { BackgroundAgentCard } from "../../BackgroundAgentCard";
import { agentFromLaunchEvent } from "../../BackgroundAgentCard/foldedAgentEvent";

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
  const threadId = useThreadId();
  const foldedEvent = useAppSelector((state) =>
    selectFoldedAgentEvent(state, threadId, toolCall.id),
  );
  const foldedAgent = useMemo(
    () =>
      foldedEvent
        ? agentFromLaunchEvent(foldedEvent, task || "Background work")
        : null,
    [foldedEvent, task],
  );

  if (foldedAgent) {
    return (
      <BackgroundAgentCard
        agent={foldedAgent}
        agentId={foldedAgent.agent_id}
        presentation="inline"
        threadId={threadId}
        toolCall={toolCall}
      />
    );
  }

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
