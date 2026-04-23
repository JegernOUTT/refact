import React, { useMemo } from "react";
import {
  CheckCircledIcon,
  CircleIcon,
  CrossCircledIcon,
  UpdateIcon,
} from "@radix-ui/react-icons";
import { Flex, Text, Box } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
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

const TaskStatusIcon: React.FC<{ status: Task["status"] }> = ({ status }) => {
  switch (status) {
    case "completed":
      return <CheckCircledIcon className={styles.completed} />;
    case "failed":
      return <CrossCircledIcon className={styles.failed} />;
    case "in_progress":
      return <UpdateIcon className={styles.inProgress} />;
    default:
      return <CircleIcon className={styles.pending} />;
  }
};

const TaskItem: React.FC<{ task: Task }> = ({ task }) => {
  return (
    <Flex align="center" gap="2" className={styles.taskItem}>
      <TaskStatusIcon status={task.status} />
      <Text size="1" className={styles[task.status]}>
        {task.content}
      </Text>
    </Flex>
  );
};

export const TasksTool: React.FC<TasksToolProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const tasks = useMemo((): Task[] => {
    try {
      const args = JSON.parse(toolCall.function.arguments) as TasksSetArgs;
      return Array.isArray(args.tasks) ? args.tasks : [];
    } catch {
      return [];
    }
  }, [toolCall.function.arguments]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    return "success";
  }, [maybeResult]);

  const stats = useMemo(() => {
    const completed = tasks.filter((t) => t.status === "completed").length;
    const total = tasks.length;
    return { completed, total };
  }, [tasks]);

  const summary = useMemo(() => {
    if (tasks.length === 0) return "Update tasks";
    return (
      <>
        Tasks{" "}
        <span className={styles.stats}>
          {stats.completed}/{stats.total}
        </span>
      </>
    );
  }, [tasks.length, stats]);

  return (
    <ToolCard
      icon={<CheckCircledIcon />}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {tasks.length > 0 && (
        <Box className={styles.taskList}>
          {tasks.map((task) => (
            <TaskItem key={task.id} task={task} />
          ))}
        </Box>
      )}
    </ToolCard>
  );
};

export default TasksTool;
