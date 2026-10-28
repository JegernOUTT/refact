import {
  Circle,
  CircleCheck,
  CircleX,
  ListChecks,
  LoaderCircle,
} from "lucide-react";
import React, { useMemo } from "react";
import { useToolCallActivity } from "./useToolCallActivity";
import { ToolCall } from "../../../services/refact/types";
import { TranscriptItem, type TranscriptStatus } from "../TranscriptItem";
import styles from "./TasksTool.module.css";

interface Task {
  id: string;
  content: string;
  status: "pending" | "in_progress" | "completed" | "failed";
}

interface TasksSetArgs {
  tasks?: Task[];
}

interface TasksToolProps {
  toolCall: ToolCall;
}

const STATUS_ICONS: Record<Task["status"], React.ReactNode> = {
  completed: <CircleCheck className={styles.completed} />,
  failed: <CircleX className={styles.failed} />,
  in_progress: <LoaderCircle className={`${styles.inProgress} rf-spin`} />,
  pending: <Circle className={styles.pending} />,
};

export const TasksTool: React.FC<TasksToolProps> = ({ toolCall }) => {
  const { result, isRunning } = useToolCallActivity(toolCall.id);

  const tasks = useMemo((): Task[] => {
    try {
      const args = JSON.parse(toolCall.function.arguments) as TasksSetArgs;
      return Array.isArray(args.tasks) ? args.tasks : [];
    } catch {
      return [];
    }
  }, [toolCall.function.arguments]);

  const failed = result?.tool_failed === true;
  const status: TranscriptStatus = isRunning
    ? "running"
    : failed
      ? "failed"
      : result
        ? "done"
        : "unknown";
  const completed = tasks.filter((t) => t.status === "completed").length;
  const current =
    tasks.find((t) => t.status === "in_progress") ??
    tasks.find((t) => t.status === "pending");

  const output =
    tasks.length > 0 ? (
      <ul className={styles.taskList}>
        {tasks.map((task) => (
          <li
            className={styles.taskItem}
            data-status={task.status}
            key={task.id}
          >
            {STATUS_ICONS[task.status]}
            <span className={styles[task.status]}>{task.content}</span>
          </li>
        ))}
      </ul>
    ) : null;

  return (
    <TranscriptItem
      data-testid="tasks-tool"
      error={
        failed && typeof result.content === "string" ? result.content : null
      }
      icon={<ListChecks />}
      meta={current?.content}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      status={status}
      statusLabel={!result && !isRunning ? "Result not recorded" : undefined}
      target={
        tasks.length > 0 ? `${completed} / ${tasks.length} done` : "update"
      }
      verb="Tasks"
    />
  );
};

export default TasksTool;
