import React, { useMemo, useState, useCallback } from "react";
import { CheckCircledIcon } from "@radix-ui/react-icons";
import { Box, Flex, Text } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import { selectToolResultById } from "../../../features/Chat/Thread/selectors";
import { ToolCall } from "../../../services/refact/types";
import { Markdown } from "../../Markdown";
import styles from "./TaskDoneTool.module.css";
import { basename } from "./utils";

interface TaskDoneResult {
  type: "task_done";
  summary: string;
  report: string;
  files_changed?: string[];
  knowledge_path?: string;
}

interface TaskDoneToolProps {
  toolCall: ToolCall;
}

export const TaskDoneTool: React.FC<TaskDoneToolProps> = ({ toolCall }) => {
  const [isOpen, setIsOpen] = useState(true);
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const handleFileClick = useCallback(
    (e: React.MouseEvent, filePath: string) => {
      e.stopPropagation();
      void queryPathThenOpenFile({ file_path: filePath });
    },
    [queryPathThenOpenFile],
  );

  const data = useMemo((): TaskDoneResult | null => {
    if (!maybeResult || typeof maybeResult.content !== "string") return null;
    try {
      return JSON.parse(maybeResult.content) as TaskDoneResult;
    } catch {
      return null;
    }
  }, [maybeResult]);

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult) return "running";
    if (maybeResult.tool_failed) return "error";
    return "success";
  }, [maybeResult]);

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  const summary = data?.summary ?? "Task completed";

  return (
    <ToolCard
      icon={<CheckCircledIcon />}
      summary={<Text className={styles.successText}>✅ {summary}</Text>}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      className={styles.taskDoneCard}
      toolCall={toolCall}
    >
      {data && (
        <Box className={styles.content}>
          <Markdown>{data.report}</Markdown>

          {data.files_changed && data.files_changed.length > 0 && (
            <Flex gap="2" wrap="wrap" mt="3" align="center">
              <Text size="1" color="gray">
                Files:
              </Text>
              {data.files_changed.map((f) => (
                <Text
                  key={f}
                  size="1"
                  className={styles.fileLink}
                  onClick={(e) => handleFileClick(e, f)}
                >
                  {basename(f)}
                </Text>
              ))}
            </Flex>
          )}

          {data.knowledge_path && (
            <Text size="1" color="gray" mt="2" as="p">
              💾 Saved to knowledge
            </Text>
          )}
        </Box>
      )}
    </ToolCard>
  );
};

export default TaskDoneTool;
