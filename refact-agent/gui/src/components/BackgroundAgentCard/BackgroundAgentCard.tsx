import { Box, Flex, Text, Badge, Link, Code } from "@radix-ui/themes";
import styles from "./BackgroundAgentCard.module.css";
import type { BackgroundAgentSummary } from "../../services/refact/types";

export interface BackgroundAgentCardProps {
  agent: BackgroundAgentSummary;
  onOpenTrajectory?: (childChatId: string) => void;
}

export const BackgroundAgentCard = ({
  agent,
  onOpenTrajectory,
}: BackgroundAgentCardProps) => {
  return (
    <Box className={styles.card} data-testid="background-agent-card">
      <Flex justify="between" align="center" gap="2" wrap="wrap">
        <Flex align="center" gap="2" wrap="wrap">
          <Badge color={statusColor(agent.status)} variant="soft">
            {statusLabel(agent.status)}
          </Badge>
          <Text weight="medium">
            {kindLabel(agent.kind)}: {agent.title}
          </Text>
        </Flex>
        <Code variant="ghost" size="1">
          {agent.agent_id}
        </Code>
      </Flex>
      <Box mt="2">
        {agent.progress && (
          <Text size="2" color="gray" as="p">
            {agent.progress}
          </Text>
        )}
        <Flex gap="3" mt="1" wrap="wrap">
          <Text size="1" color="gray">
            Steps: {agent.step_count}
          </Text>
          {agent.last_activity && (
            <Text size="1" color="gray">
              Last: {agent.last_activity}
            </Text>
          )}
        </Flex>
      </Box>
      {agent.target_files.length > 0 && (
        <Box mt="2">
          <Text size="1" color="gray">
            Target files:
          </Text>
          <ul className={styles.fileList}>
            {agent.target_files.map((file) => (
              <li key={file}>
                <Code size="1">{file}</Code>
              </li>
            ))}
          </ul>
        </Box>
      )}
      {agent.edited_files.length > 0 && (
        <Box mt="2">
          <Text size="1" color="gray">
            Edited:
          </Text>
          <ul className={styles.fileList}>
            {agent.edited_files.map((file) => (
              <li key={file}>
                <Code size="1">{file}</Code>
              </li>
            ))}
          </ul>
        </Box>
      )}
      {agent.conflict_summary && (
        <Box mt="2">
          <Flex gap="2" align="center" wrap="wrap">
            <Badge color="amber">⚠ conflicts</Badge>
            <Text size="1">{agent.conflict_summary}</Text>
          </Flex>
        </Box>
      )}
      {agent.error && (
        <Box mt="2">
          <Flex gap="2" align="center" wrap="wrap">
            <Badge color="red">error</Badge>
            <Text size="1">{agent.error}</Text>
          </Flex>
        </Box>
      )}
      {agent.child_chat_id && onOpenTrajectory && (
        <Box mt="2">
          <Link
            size="1"
            role="button"
            tabIndex={0}
            onClick={() => {
              if (agent.child_chat_id) onOpenTrajectory(agent.child_chat_id);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                if (agent.child_chat_id) onOpenTrajectory(agent.child_chat_id);
              }
            }}
          >
            Open child trajectory
          </Link>
        </Box>
      )}
    </Box>
  );
};

function statusColor(
  status: BackgroundAgentSummary["status"],
): "gray" | "blue" | "amber" | "green" | "red" | "tomato" {
  switch (status) {
    case "queued":
      return "gray";
    case "running":
      return "blue";
    case "waiting_for_approval":
      return "amber";
    case "completed":
      return "green";
    case "failed":
      return "red";
    case "cancelled":
      return "tomato";
    case "interrupted":
      return "amber";
  }
}

function statusLabel(status: BackgroundAgentSummary["status"]): string {
  return status.replace(/_/g, " ");
}

function kindLabel(kind: BackgroundAgentSummary["kind"]): string {
  return kind === "delegate" ? "Delegate" : "Subagent";
}
