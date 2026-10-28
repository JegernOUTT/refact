import { FileText } from "lucide-react";
import React from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard, type ReportData } from "./ReportToolCard";
import { readCompactRecord } from "../../Tools/AgenticTool/compactPresentation";
import { useIsLatestFinalReport } from "../finalReportLatest";

interface TaskDoneToolProps {
  toolCall: ToolCall;
  latest?: boolean;
}

function extractTaskDoneReport(content: string): ReportData | null {
  const data = readCompactRecord(content);
  if (data?.type !== "task_done") return null;
  return {
    summary:
      typeof data.summary === "string" ? data.summary : "Task report received",
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

export const TaskDoneTool: React.FC<TaskDoneToolProps> = ({
  toolCall,
  latest,
}) => {
  const isLatestInThread = useIsLatestFinalReport(toolCall.id);
  return (
    <ReportToolCard
      defaultOpen={latest ?? isLatestInThread}
      defaultSummary="Task report"
      extractReport={extractTaskDoneReport}
      icon={<FileText />}
      testId="task-done-tool"
      toolCall={toolCall}
      variant="taskDone"
    />
  );
};

export default TaskDoneTool;
