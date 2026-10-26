import { CircleCheck } from "lucide-react";
import React from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard, type ReportData } from "./ReportToolCard";
import { readCompactRecord } from "../../Tools/AgenticTool/compactPresentation";

interface TaskDoneToolProps {
  toolCall: ToolCall;
}

function extractTaskDoneReport(content: string): ReportData | null {
  const data = readCompactRecord(content);
  if (data?.type !== "task_done") return null;
  return {
    summary: typeof data.summary === "string" ? data.summary : "Task completed",
    markdown: typeof data.report === "string" ? data.report : content,
    filesChanged:
      Array.isArray(data.files_changed) &&
      data.files_changed.every((file) => typeof file === "string")
        ? data.files_changed
        : undefined,
    knowledgePath:
      typeof data.knowledge_path === "string" ? data.knowledge_path : undefined,
    success: typeof data.success === "boolean" ? data.success : undefined,
  };
}

export const TaskDoneTool: React.FC<TaskDoneToolProps> = ({ toolCall }) => {
  return (
    <>
      <span data-testid="task-done-tool" hidden />
      <ReportToolCard
        toolCall={toolCall}
        icon={<CircleCheck />}
        defaultSummary="Task completed"
        variant="taskDone"
        extractReport={extractTaskDoneReport}
        defaultOpen
        unboundedContent
      />
    </>
  );
};

export default TaskDoneTool;
