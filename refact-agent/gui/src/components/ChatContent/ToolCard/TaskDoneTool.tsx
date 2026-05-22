import React from "react";
import { CheckCircledIcon } from "@radix-ui/react-icons";
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
    <div data-testid="task-done-tool" style={{ display: "contents" }}>
    <ReportToolCard
      toolCall={toolCall}
      icon={<CheckCircledIcon />}
      defaultSummary="Task completed"
      variant="taskDone"
      extractReport={extractTaskDoneReport}
      defaultOpen={false}
    />
    </div>
  );
};

export default TaskDoneTool;
