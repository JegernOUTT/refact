import { CircleCheck } from "lucide-react";
import React from "react";
import { ToolCall } from "../../../services/refact/types";
import { ReportToolCard, type ReportData } from "./ReportToolCard";

interface TaskDoneToolProps {
  toolCall: ToolCall;
}

function extractTaskDoneReport(content: string): ReportData | null {
  try {
    const data = JSON.parse(content) as {
      type?: string;
      summary?: string;
      report?: string;
      files_changed?: string[];
      knowledge_path?: string;
    };
    if (data.type !== "task_done") return null;
    return {
      summary: data.summary ?? "Task completed",
      markdown: data.report ?? content,
      filesChanged: data.files_changed,
      knowledgePath: data.knowledge_path,
    };
  } catch {
    return null;
  }
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
