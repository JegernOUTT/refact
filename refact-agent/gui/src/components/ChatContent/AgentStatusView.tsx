import React, { useCallback, useMemo, useState } from "react";
import {
  Badge,
  Box,
  Button,
  Callout,
  Dialog,
  Flex,
  Select,
  Spinner,
  Table,
  Tabs,
  Text,
  TextArea,
  TextField,
} from "@radix-ui/themes";
import { ExclamationTriangleIcon, GearIcon } from "@radix-ui/react-icons";
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
  STATUS_TABS,
  countAgentAlerts,
  filterAgentStatusRows,
  formatAgentActionCommand,
  mergeAgentAlerts,
  parseAgentStatusOutput,
  type AgeFilter,
  type AgentStatusReport,
  type AgentStatusRow,
  type AgentStatusState,
  type AgentStatusTab,
  type PriorityFilter,
} from "./AgentStatusModel";
import styles from "./AgentStatusView.module.css";

const EMPTY_ALERTS = { stuck: 0, failed: 0, paused: 0 };

type AgentStatusContentProps = {
  report: AgentStatusReport;
  onSubmitCommand?: (command: string) => void | Promise<void>;
  actionsDisabled?: boolean;
};

type AgentStatusViewProps = {
  toolCall: ToolCall;
};

type DialogState =
  | { kind: "queued"; title: string; command: string }
  | { kind: "steer"; row: AgentStatusRow }
  | { kind: "cancel"; row: AgentStatusRow }
  | null;

function isStatusTab(value: string): value is AgentStatusTab {
  return STATUS_TABS.includes(value as AgentStatusTab);
}

function priorityBadgeColor(
  priority: string,
): "red" | "amber" | "blue" | "gray" {
  switch (priority) {
    case "P0":
      return "red";
    case "P1":
      return "amber";
    case "P2":
      return "blue";
    default:
      return "gray";
  }
}

function stateClass(state: AgentStatusState): string {
  switch (state) {
    case "stuck":
      return styles.stateStuck;
    case "failed":
      return styles.stateFailed;
    case "done":
      return styles.stateDone;
    case "paused":
      return styles.statePaused;
    case "running":
      return styles.stateRunning;
  }
}

function truncateText(text: string, limit: number): string {
  if (text.length <= limit) return text;
  return `${text.slice(0, limit - 1)}…`;
}

function tabLabel(tab: AgentStatusTab): string {
  switch (tab) {
    case "all":
      return "All";
    case "running":
      return "Running";
    case "stuck":
      return "Stuck";
    case "failed":
      return "Failed";
    case "done":
      return "Done";
    case "paused":
      return "Paused";
  }
}

function tabCount(rows: AgentStatusRow[], tab: AgentStatusTab): number {
  if (tab === "all") return rows.length;
  return rows.filter((row) => row.state === tab).length;
}

function renderDetailValue(
  value: string | null,
  empty: string,
): React.ReactNode {
  if (!value) return <Text color="gray">{empty}</Text>;
  return value;
}

export const AgentStatusContent: React.FC<AgentStatusContentProps> = ({
  report,
  onSubmitCommand,
  actionsDisabled = false,
}) => {
  const [tab, setTab] = useState<AgentStatusTab>("all");
  const [priority, setPriority] = useState<PriorityFilter>("all");
  const [ageFilter, setAgeFilter] = useState<AgeFilter>("all");
  const [expandedRows, setExpandedRows] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [dialog, setDialog] = useState<DialogState>(null);
  const [steerMessage, setSteerMessage] = useState("");
  const [cancelReason, setCancelReason] = useState(DEFAULT_CANCEL_REASON);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const alerts = useMemo(
    () => mergeAgentAlerts(report.alerts, countAgentAlerts(report.rows)),
    [report.alerts, report.rows],
  );
  const alertCount = alerts.stuck + alerts.failed + alerts.paused;
  const minAgeMinutes = ageFilter === "all" ? null : Number(ageFilter);

  const visibleRows = useMemo(
    () => filterAgentStatusRows(report.rows, { tab, priority, minAgeMinutes }),
    [report.rows, tab, priority, minAgeMinutes],
  );

  const handleTabChange = useCallback((value: string) => {
    if (isStatusTab(value)) setTab(value);
  }, []);

  const handlePriorityChange = useCallback((value: string) => {
    if (value === "all" || value === "P0" || value === "P1" || value === "P2") {
      setPriority(value);
    }
  }, []);

  const handleAgeChange = useCallback((value: string) => {
    if (
      value === "all" ||
      value === "15" ||
      value === "60" ||
      value === "240"
    ) {
      setAgeFilter(value);
    }
  }, []);

  const toggleExpanded = useCallback((cardId: string) => {
    setExpandedRows((previous) => {
      const next = new Set(previous);
      if (next.has(cardId)) {
        next.delete(cardId);
      } else {
        next.add(cardId);
      }
      return next;
    });
  }, []);

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

  const openSteerDialog = useCallback((row: AgentStatusRow) => {
    setSteerMessage("");
    setDialogError(null);
    setDialog({ kind: "steer", row });
  }, []);

  const openCancelDialog = useCallback((row: AgentStatusRow) => {
    setCancelReason(DEFAULT_CANCEL_REASON);
    setDialogError(null);
    setDialog({ kind: "cancel", row });
  }, []);

  const submitSteer = useCallback(() => {
    if (!dialog || dialog.kind !== "steer") return;
    const message = steerMessage.trim();
    if (!message) return;
    void submitCommand(
      `Steer ${dialog.row.cardId}`,
      formatAgentActionCommand("steer", dialog.row.cardId, message),
    );
  }, [dialog, steerMessage, submitCommand]);

  const submitCancel = useCallback(() => {
    if (!dialog || dialog.kind !== "cancel") return;
    void submitCommand(
      `Cancel ${dialog.row.cardId}`,
      formatAgentActionCommand(
        "cancel",
        dialog.row.cardId,
        cancelReason.trim(),
      ),
    );
  }, [cancelReason, dialog, submitCommand]);

  const closeDialog = useCallback(() => {
    if (!isSubmitting) setDialog(null);
  }, [isSubmitting]);

  return (
    <Box className={styles.root}>
      {alertCount > 0 && (
        <Box className={styles.stickyAlerts}>
          <Callout.Root color={alerts.failed > 0 ? "red" : "amber"} size="1">
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>
              {alerts.stuck} stuck, {alerts.failed} failed, {alerts.paused}{" "}
              needing approval
            </Callout.Text>
          </Callout.Root>
        </Box>
      )}

      <Flex direction="column" gap="2">
        <Tabs.Root value={tab} onValueChange={handleTabChange}>
          <Tabs.List size="1" className={styles.tabsList}>
            {STATUS_TABS.map((item) => (
              <Tabs.Trigger key={item} value={item}>
                {tabLabel(item)} {tabCount(report.rows, item)}
              </Tabs.Trigger>
            ))}
          </Tabs.List>
        </Tabs.Root>

        <Flex gap="2" wrap="wrap" align="center" className={styles.filters}>
          <Text size="1" color="gray">
            Priority
          </Text>
          <Select.Root
            value={priority}
            onValueChange={handlePriorityChange}
            size="1"
          >
            <Select.Trigger aria-label="Priority filter" />
            <Select.Content>
              <Select.Item value="all">All priorities</Select.Item>
              <Select.Item value="P0">P0</Select.Item>
              <Select.Item value="P1">P1</Select.Item>
              <Select.Item value="P2">P2</Select.Item>
            </Select.Content>
          </Select.Root>

          <Text size="1" color="gray">
            Age
          </Text>
          <Select.Root
            value={ageFilter}
            onValueChange={handleAgeChange}
            size="1"
          >
            <Select.Trigger aria-label="Age filter" />
            <Select.Content>
              <Select.Item value="all">Any age</Select.Item>
              <Select.Item value="15">15m+</Select.Item>
              <Select.Item value="60">1h+</Select.Item>
              <Select.Item value="240">4h+</Select.Item>
            </Select.Content>
          </Select.Root>
        </Flex>

        <Box className={styles.tableWrap}>
          <Table.Root size="1" variant="surface" className={styles.table}>
            <Table.Header>
              <Table.Row>
                <Table.ColumnHeaderCell>Priority</Table.ColumnHeaderCell>
                <Table.ColumnHeaderCell>Card</Table.ColumnHeaderCell>
                <Table.ColumnHeaderCell>Title</Table.ColumnHeaderCell>
                <Table.ColumnHeaderCell>State</Table.ColumnHeaderCell>
                <Table.ColumnHeaderCell>Age</Table.ColumnHeaderCell>
                <Table.ColumnHeaderCell>Last-tool</Table.ColumnHeaderCell>
                <Table.ColumnHeaderCell>Actions</Table.ColumnHeaderCell>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              {visibleRows.map((row) => {
                const isExpanded = expandedRows.has(row.cardId);
                return (
                  <React.Fragment key={row.cardId}>
                    <Table.Row>
                      <Table.Cell>
                        <Badge
                          color={priorityBadgeColor(row.priority)}
                          variant="soft"
                        >
                          {row.priority}
                        </Badge>
                      </Table.Cell>
                      <Table.Cell>
                        <Text asChild size="1" weight="medium">
                          <a
                            href={`#${row.cardId}`}
                            className={styles.cardLink}
                          >
                            {row.cardId}
                          </a>
                        </Text>
                      </Table.Cell>
                      <Table.Cell className={styles.titleCell}>
                        {row.title}
                      </Table.Cell>
                      <Table.Cell>
                        <Text
                          className={classNames(
                            styles.stateText,
                            stateClass(row.state),
                          )}
                        >
                          {row.emoji} {row.stateText}
                        </Text>
                      </Table.Cell>
                      <Table.Cell>{row.age}</Table.Cell>
                      <Table.Cell>{row.lastTool ?? "—"}</Table.Cell>
                      <Table.Cell>
                        <Flex gap="1" wrap="wrap" className={styles.actions}>
                          <Button
                            size="1"
                            variant="ghost"
                            onClick={() => toggleExpanded(row.cardId)}
                            aria-label={`Toggle details ${row.cardId}`}
                          >
                            {isExpanded ? "▾" : "▸"}
                          </Button>
                          <Button
                            size="1"
                            variant="soft"
                            disabled={actionsDisabled || isSubmitting}
                            onClick={() => {
                              void submitCommand(
                                `View pulse ${row.cardId}`,
                                formatAgentActionCommand("pulse", row.cardId),
                              );
                            }}
                            aria-label={`View pulse ${row.cardId}`}
                          >
                            🔍
                          </Button>
                          <Button
                            size="1"
                            variant="soft"
                            disabled={actionsDisabled || isSubmitting}
                            onClick={() => {
                              void submitCommand(
                                `View diff ${row.cardId}`,
                                formatAgentActionCommand("diff", row.cardId),
                              );
                            }}
                            aria-label={`View diff ${row.cardId}`}
                          >
                            📋
                          </Button>
                          <Button
                            size="1"
                            variant="soft"
                            disabled={actionsDisabled || isSubmitting}
                            onClick={() => openSteerDialog(row)}
                            aria-label={`Steer ${row.cardId}`}
                          >
                            ✋
                          </Button>
                          <Button
                            size="1"
                            variant="soft"
                            color="red"
                            disabled={actionsDisabled || isSubmitting}
                            onClick={() => openCancelDialog(row)}
                            aria-label={`Cancel agent ${row.cardId}`}
                          >
                            🛑
                          </Button>
                        </Flex>
                      </Table.Cell>
                    </Table.Row>
                    {isExpanded && (
                      <Table.Row>
                        <Table.Cell colSpan={7} className={styles.detailsCell}>
                          <Flex
                            direction="column"
                            gap="2"
                            className={styles.details}
                          >
                            <Box>
                              <Text size="1" color="gray" as="div">
                                Last status update
                              </Text>
                              <Text size="2" as="div">
                                {renderDetailValue(
                                  row.lastStatusUpdate,
                                  "Not included in compact output.",
                                )}
                              </Text>
                            </Box>
                            <Box>
                              <Text size="1" color="gray" as="div">
                                Final report excerpt
                              </Text>
                              <Text size="2" as="div">
                                {renderDetailValue(
                                  row.finalReport
                                    ? truncateText(row.finalReport, 300)
                                    : null,
                                  "No final report in this output.",
                                )}
                              </Text>
                            </Box>
                          </Flex>
                        </Table.Cell>
                      </Table.Row>
                    )}
                  </React.Fragment>
                );
              })}
            </Table.Body>
          </Table.Root>
        </Box>

        {visibleRows.length === 0 && (
          <Text size="2" color="gray" className={styles.emptyState}>
            No agents match the selected filters.
          </Text>
        )}
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
              <Box className={styles.commandPreview}>{dialog.command}</Box>
            </>
          )}

          {dialog?.kind === "steer" && (
            <>
              <Dialog.Title>Steer {dialog.row.cardId}</Dialog.Title>
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
              <Dialog.Title>Cancel {dialog.row.cardId}</Dialog.Title>
              <Dialog.Description size="2" color="gray">
                Confirm cancellation and optionally edit the reason.
              </Dialog.Description>
              <TextField.Root
                aria-label="Cancel reason"
                value={cancelReason}
                onChange={(event) => setCancelReason(event.target.value)}
              />
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
              {dialog?.kind === "queued" ? "Close" : "Cancel"}
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
                onClick={submitCancel}
                disabled={isSubmitting}
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

export const AgentStatusView: React.FC<AgentStatusViewProps> = ({
  toolCall,
}) => {
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
    () => (content ? parseAgentStatusOutput(content) : null),
    [content],
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    return maybeResult.tool_failed ? "error" : "success";
  }, [isStreaming, isWaiting, maybeResult]);

  const alerts = report
    ? mergeAgentAlerts(report.alerts, countAgentAlerts(report.rows))
    : EMPTY_ALERTS;
  const alertCount = alerts.stuck + alerts.failed + alerts.paused;
  const summary = report
    ? `Check agents: ${report.rows.length} agents`
    : "Check agents";
  const meta = report && alertCount > 0 ? `${alertCount} alerts` : undefined;

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
    <div data-testid="agent-status-view" style={{ display: "contents" }}>
    <ToolCard
      icon={<GearIcon />}
      summary={summary}
      meta={meta}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {report ? (
        <AgentStatusContent
          report={report}
          onSubmitCommand={handleSubmitCommand}
          actionsDisabled={!chatId || !port}
        />
      ) : content ? (
        <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
      ) : null}
    </ToolCard>
    </div>
  );
};

export default AgentStatusView;
