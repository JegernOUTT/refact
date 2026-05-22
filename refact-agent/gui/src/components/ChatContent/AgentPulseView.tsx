import React, { useCallback, useMemo, useState } from "react";
import {
  Badge,
  Box,
  Button,
  Callout,
  Dialog,
  Flex,
  Spinner,
  Text,
  TextArea,
} from "@radix-ui/themes";
import { ExclamationTriangleIcon, LapTimerIcon } from "@radix-ui/react-icons";
import classNames from "classnames";
import { useAppSelector } from "../../hooks";
import {
  selectChatId,
  selectIsStreaming,
  selectIsWaiting,
  selectToolResultById,
} from "../../features/Chat/Thread/selectors";
import { selectApiKey, selectLspPort } from "../../features/Config/configSlice";
import { sendChatCommand } from "../../services/refact/chatCommands";
import type { ToolCall } from "../../services/refact/types";
import { ShikiCodeBlock } from "../Markdown";
import { ToolCard, type ToolStatus } from "./ToolCard";
import { useStoredOpen } from "./useStoredOpen";
import {
  DEFAULT_CANCEL_REASON,
  DEFAULT_PAUSE_REASON,
  formatAgentActionCommand,
} from "./AgentStatusModel";
import {
  parseAgentPulseOutput,
  type AgentPulseReport,
  type AgentPulseState,
} from "./AgentPulseModel";
import styles from "./AgentPulseView.module.css";

type AgentPulseContentProps = {
  report: AgentPulseReport;
  onSubmitCommand?: (command: string) => void | Promise<void>;
  actionsDisabled?: boolean;
};

type AgentPulseViewProps = {
  toolCall: ToolCall;
};

type DialogState =
  | { kind: "queued"; title: string; command: string }
  | { kind: "steer" }
  | { kind: "cancel" }
  | null;

function stateClass(state: AgentPulseState): string {
  switch (state) {
    case "running":
      return styles.stateRunning;
    case "paused":
      return styles.statePaused;
    case "waiting":
      return styles.stateWaiting;
    case "done":
      return styles.stateDone;
    case "error":
      return styles.stateError;
    case "idle":
      return styles.stateIdle;
    case "unknown":
      return styles.stateUnknown;
  }
}

function maybeValue(value: string): string {
  return value && value !== "unknown" ? value : "—";
}

export const AgentPulseContent: React.FC<AgentPulseContentProps> = ({
  report,
  onSubmitCommand,
  actionsDisabled = false,
}) => {
  const [dialog, setDialog] = useState<DialogState>(null);
  const [steerMessage, setSteerMessage] = useState("");
  const [dialogError, setDialogError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const submitCommand = useCallback(
    async (title: string, command: string) => {
      setDialog({ kind: "queued", title, command });
      setDialogError(null);
      setIsSubmitting(true);
      try {
        await onSubmitCommand?.(command);
      } catch (error) {
        setDialogError(error instanceof Error ? error.message : String(error));
      } finally {
        setIsSubmitting(false);
      }
    },
    [onSubmitCommand],
  );

  const openSteer = useCallback(() => {
    setSteerMessage("");
    setDialogError(null);
    setDialog({ kind: "steer" });
  }, []);

  const openCancel = useCallback(() => {
    setDialogError(null);
    setDialog({ kind: "cancel" });
  }, []);

  const closeDialog = useCallback(() => {
    if (!isSubmitting) setDialog(null);
  }, [isSubmitting]);

  const submitSteer = useCallback(() => {
    const message = steerMessage.trim();
    if (!message) return;
    void submitCommand(
      `Steer ${report.cardId}`,
      formatAgentActionCommand("steer", report.cardId, message),
    );
  }, [report.cardId, steerMessage, submitCommand]);

  return (
    <Box className={styles.root}>
      <Box className={styles.header}>
        <Flex justify="between" align="center" gap="2">
          <Text weight="medium" className={styles.title}>
            Pulse: {report.cardId}
          </Text>
          <Badge className={styles.stateBadge} variant="soft">
            <Text
              as="span"
              size="1"
              className={classNames(
                styles.stateText,
                stateClass(report.stateKind),
              )}
            >
              {report.state}
            </Text>
          </Badge>
        </Flex>

        <Box className={styles.metaGrid}>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Tokens</span>
            <span className={styles.value}>{maybeValue(report.tokens)}</span>
          </Box>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Last activity</span>
            <span className={styles.value}>
              {maybeValue(report.lastActivity)}
            </span>
          </Box>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Editing</span>
            <span className={styles.value}>
              {maybeValue(report.currentlyEditing)}
            </span>
          </Box>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Card</span>
            <span className={styles.value}>{report.cardTitle}</span>
          </Box>
        </Box>

        {report.sessionNote && (
          <Text size="1" as="div" className={styles.note}>
            {report.sessionNote}
          </Text>
        )}
      </Box>

      <Box className={styles.section}>
        <Text as="div" className={styles.sectionLabel}>
          Last assistant message
        </Text>
        <Text as="div" size="2" className={styles.quote}>
          {report.lastAssistantMessage}
        </Text>
      </Box>

      <Box className={styles.section}>
        <Text as="div" className={styles.sectionLabel}>
          Last tool
        </Text>
        <Text as="div" size="2" className={styles.toolCall}>
          {report.lastToolCall}
        </Text>
      </Box>

      <Flex gap="2" wrap="wrap" className={styles.actions}>
        <Button
          size="1"
          variant="soft"
          disabled={actionsDisabled || isSubmitting}
          onClick={openSteer}
        >
          Steer
        </Button>
        <Button
          size="1"
          variant="soft"
          disabled={actionsDisabled || isSubmitting}
          onClick={() => {
            void submitCommand(
              `Pause ${report.cardId}`,
              formatAgentActionCommand(
                "pause",
                report.cardId,
                DEFAULT_PAUSE_REASON,
              ),
            );
          }}
        >
          Pause
        </Button>
        <Button
          size="1"
          variant="soft"
          color="red"
          disabled={actionsDisabled || isSubmitting}
          onClick={openCancel}
        >
          Cancel
        </Button>
        <Button
          size="1"
          variant="soft"
          disabled={actionsDisabled || isSubmitting}
          onClick={() => {
            void submitCommand(
              `View diff ${report.cardId}`,
              formatAgentActionCommand("diff", report.cardId),
            );
          }}
        >
          Diff
        </Button>
      </Flex>

      <Dialog.Root
        open={dialog !== null}
        onOpenChange={(open) => !open && closeDialog()}
      >
        <Dialog.Content className={styles.dialogContent}>
          {dialog?.kind === "queued" && (
            <>
              <Dialog.Title>{dialog.title}</Dialog.Title>
              <Dialog.Description size="2" color="gray">
                The command was sent through the chat queue.
              </Dialog.Description>
              <Box className={styles.toolCall}>{dialog.command}</Box>
            </>
          )}

          {dialog?.kind === "steer" && (
            <>
              <Dialog.Title>Steer {report.cardId}</Dialog.Title>
              <Dialog.Description size="2" color="gray">
                Send a planner steering message to this agent.
              </Dialog.Description>
              <TextArea
                aria-label="Steering message"
                value={steerMessage}
                onChange={(event) => setSteerMessage(event.target.value)}
                placeholder="Add guidance for the agent"
                className={styles.dialogInput}
              />
            </>
          )}

          {dialog?.kind === "cancel" && (
            <>
              <Dialog.Title>Cancel {report.cardId}</Dialog.Title>
              <Dialog.Description size="2" color="gray">
                Send a cancellation command with the default reason.
              </Dialog.Description>
              <Box className={styles.toolCall}>
                {formatAgentActionCommand(
                  "cancel",
                  report.cardId,
                  DEFAULT_CANCEL_REASON,
                )}
              </Box>
            </>
          )}

          {dialogError && (
            <Callout.Root color="red" size="1">
              <Callout.Icon>
                <ExclamationTriangleIcon />
              </Callout.Icon>
              <Callout.Text>{dialogError}</Callout.Text>
            </Callout.Root>
          )}

          <Flex gap="2" justify="end" mt="3">
            <Button
              variant="soft"
              color="gray"
              onClick={closeDialog}
              disabled={isSubmitting}
            >
              {dialog?.kind === "queued" ? "Close" : "Back"}
            </Button>
            {dialog?.kind === "steer" && (
              <Button
                onClick={submitSteer}
                disabled={isSubmitting || !steerMessage.trim()}
              >
                {isSubmitting ? <Spinner size="1" /> : "Send steer"}
              </Button>
            )}
            {dialog?.kind === "cancel" && (
              <Button
                color="red"
                disabled={isSubmitting}
                onClick={() => {
                  void submitCommand(
                    `Cancel ${report.cardId}`,
                    formatAgentActionCommand(
                      "cancel",
                      report.cardId,
                      DEFAULT_CANCEL_REASON,
                    ),
                  );
                }}
              >
                {isSubmitting ? <Spinner size="1" /> : "Confirm cancel"}
              </Button>
            )}
          </Flex>
        </Dialog.Content>
      </Dialog.Root>
    </Box>
  );
};

export const AgentPulseView: React.FC<AgentPulseViewProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const chatId = useAppSelector(selectChatId);
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const report = useMemo(
    () => (content ? parseAgentPulseOutput(content) : null),
    [content],
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    return maybeResult.tool_failed ? "error" : "success";
  }, [isStreaming, isWaiting, maybeResult]);

  const handleSubmitCommand = useCallback(
    async (command: string) => {
      await sendChatCommand(
        chatId,
        port,
        apiKey ?? undefined,
        { type: "user_message", content: command },
        true,
      );
    },
    [apiKey, chatId, port],
  );

  return (
    <>
      <span data-testid="agent-pulse-view" hidden />
      <ToolCard
        icon={<LapTimerIcon />}
        summary={report ? `Agent pulse: ${report.cardId}` : "Agent pulse"}
        meta={report?.state}
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        {report ? (
          <AgentPulseContent
            report={report}
            onSubmitCommand={handleSubmitCommand}
            actionsDisabled={!chatId || !port}
          />
        ) : content ? (
          <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
        ) : null}
      </ToolCard>
    </>
  );
};

export default AgentPulseView;
