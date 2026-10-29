import React, { useCallback, useMemo, useState } from "react";
import classNames from "classnames";
import {
  CircleAlert,
  CircleStop,
  FileDiff,
  Hand,
  LoaderCircle,
  Pause,
  Send,
  Timer,
} from "lucide-react";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectToolResultByThreadAndId } from "../../features/Chat/Thread/selectors";
import { useThreadId } from "../../features/Chat/Thread";
import { selectApiKey, selectConfig } from "../../features/Config/configSlice";
import { sendChatCommand } from "../../services/refact/chatCommands";
import type { ToolCall } from "../../services/refact/types";
import {
  COMPACT_HEADLINE_CHARS,
  compactText,
} from "../Tools/AgenticTool/compactPresentation";
import { Badge, Button, Dialog, FieldTextarea, Icon, StatusDot } from "../ui";
import { useUnrecordedResultState } from "./TranscriptItem/useUnrecordedResultState";
import { TranscriptItem, type TranscriptStatus } from "./TranscriptItem";
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

function stateStatus(
  state: AgentPulseState,
): React.ComponentProps<typeof StatusDot>["status"] {
  switch (state) {
    case "running":
    case "waiting":
      return "running";
    case "paused":
      return "warning";
    case "done":
      return "success";
    case "error":
      return "error";
    case "idle":
    case "unknown":
      return "idle";
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
    <div className={styles.root}>
      <header className={classNames(styles.header, "rf-enter")}>
        <div className={styles.headerTop}>
          <span className={styles.title}>Pulse: {report.cardId}</span>
          <Badge className={styles.stateBadge} tone="accent">
            <StatusDot status={stateStatus(report.stateKind)} />
            <span
              className={classNames(
                styles.stateText,
                stateClass(report.stateKind),
              )}
            >
              {report.state}
            </span>
          </Badge>
        </div>

        <div className={styles.metaGrid}>
          <div className={styles.metaItem}>
            <span className={styles.label}>Tokens</span>
            <span className={styles.value}>{maybeValue(report.tokens)}</span>
          </div>
          <div className={styles.metaItem}>
            <span className={styles.label}>Last activity</span>
            <span className={styles.value}>
              {maybeValue(report.lastActivity)}
            </span>
          </div>
          <div className={styles.metaItem}>
            <span className={styles.label}>Editing</span>
            <span className={styles.value}>
              {maybeValue(report.currentlyEditing)}
            </span>
          </div>
          <div className={styles.metaItem}>
            <span className={styles.label}>Card</span>
            <span className={styles.value}>{report.cardTitle}</span>
          </div>
        </div>

        {report.sessionNote && (
          <div className={styles.note}>{report.sessionNote}</div>
        )}
      </header>

      <section className={styles.section}>
        <span className={styles.sectionLabel}>Last assistant message</span>
        <div className={styles.quote}>{report.lastAssistantMessage}</div>
      </section>

      <section className={styles.section}>
        <span className={styles.sectionLabel}>Last tool</span>
        <div className={styles.toolCall}>{report.lastToolCall}</div>
      </section>

      <div className={styles.actions}>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={Hand}
          disabled={actionsDisabled || isSubmitting}
          onClick={openSteer}
        >
          Steer
        </Button>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={Pause}
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
          size="sm"
          variant="danger"
          leftIcon={CircleStop}
          disabled={actionsDisabled || isSubmitting}
          onClick={openCancel}
        >
          Cancel
        </Button>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={FileDiff}
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
      </div>

      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => !open && closeDialog()}
      >
        <Dialog.Content className={styles.dialogContent}>
          {dialog?.kind === "queued" && (
            <>
              <Dialog.Title>{dialog.title}</Dialog.Title>
              <Dialog.Description>
                The command was sent through the chat queue.
              </Dialog.Description>
              <div className={styles.toolCall}>{dialog.command}</div>
            </>
          )}

          {dialog?.kind === "steer" && (
            <>
              <Dialog.Title>Steer {report.cardId}</Dialog.Title>
              <Dialog.Description>
                Send a planner steering message to this agent.
              </Dialog.Description>
              <FieldTextarea
                aria-label="Steering message"
                value={steerMessage}
                onChange={setSteerMessage}
                placeholder="Add guidance for the agent"
                className={styles.dialogInput}
              />
            </>
          )}

          {dialog?.kind === "cancel" && (
            <>
              <Dialog.Title>Cancel {report.cardId}</Dialog.Title>
              <Dialog.Description>
                Send a cancellation command with the default reason.
              </Dialog.Description>
              <div className={styles.toolCall}>
                {formatAgentActionCommand(
                  "cancel",
                  report.cardId,
                  DEFAULT_CANCEL_REASON,
                )}
              </div>
            </>
          )}

          {dialogError && (
            <div className={styles.alert}>
              <Icon icon={CircleAlert} size="sm" tone="danger" />
              <span>{dialogError}</span>
            </div>
          )}

          <div className={styles.dialogActions}>
            <Button
              variant="soft"
              onClick={closeDialog}
              disabled={isSubmitting}
            >
              {dialog?.kind === "queued" ? "Close" : "Back"}
            </Button>
            {dialog?.kind === "steer" && (
              <Button
                variant="primary"
                leftIcon={isSubmitting ? LoaderCircle : Send}
                onClick={submitSteer}
                disabled={isSubmitting || !steerMessage.trim()}
              >
                Send steer
              </Button>
            )}
            {dialog?.kind === "cancel" && (
              <Button
                variant="danger"
                leftIcon={isSubmitting ? LoaderCircle : CircleStop}
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
                Confirm cancel
              </Button>
            )}
          </div>
        </Dialog.Content>
      </Dialog>
    </div>
  );
};

export const AgentPulseView: React.FC<AgentPulseViewProps> = ({ toolCall }) => {
  const chatId = useThreadId();
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);

  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, chatId, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const report = useMemo(
    () => (content ? parseAgentPulseOutput(content) : null),
    [content],
  );

  const failed = maybeResult?.tool_failed === true;
  const status: TranscriptStatus = !maybeResult
    ? unrecorded.status
    : failed || report?.stateKind === "error"
      ? "failed"
      : report
        ? "done"
        : "unknown";
  const title =
    report && report.cardTitle !== report.cardId
      ? compactText(report.cardTitle, COMPACT_HEADLINE_CHARS)
      : report?.cardId ?? null;
  const facts = report
    ? [
        { label: "State", value: report.state },
        { label: "Last activity", value: report.lastActivity },
        { label: "Editing", value: report.currentlyEditing },
      ].filter((fact) => fact.value && fact.value !== "unknown")
    : [];

  const handleSubmitCommand = useCallback(
    async (command: string) => {
      await sendChatCommand(
        chatId,
        config,
        apiKey ?? undefined,
        { type: "user_message", content: command },
        true,
      );
    },
    [apiKey, chatId, config],
  );

  return (
    <TranscriptItem
      data-testid="agent-pulse-view"
      icon={<Timer aria-hidden />}
      verb="Pulse"
      target={title ?? undefined}
      status={status}
      statusLabel={!maybeResult ? unrecorded.statusLabel : undefined}
      meta={maybeResult && !failed && report ? report.state : undefined}
      summary={facts}
      output={
        report ? (
          <AgentPulseContent
            report={report}
            onSubmitCommand={handleSubmitCommand}
            actionsDisabled={!chatId}
          />
        ) : undefined
      }
      error={failed ? content : undefined}
      raw={[
        { label: "Arguments", value: toolCall.function.arguments },
        ...(content !== null ? [{ label: "Response", value: content }] : []),
      ]}
      openKey={toolCall.id ? `agent_pulse:${toolCall.id}` : undefined}
    />
  );
};

export default AgentPulseView;
