import React, { useCallback, useMemo, useState } from "react";
import classNames from "classnames";
import {
  CircleAlert,
  CircleStop,
  ClipboardList,
  Eye,
  FileDiff,
  Hand,
  LoaderCircle,
  Send,
  Settings,
} from "lucide-react";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectToolResultByThreadAndId,
} from "../../features/Chat/Thread/selectors";
import { useThreadId } from "../../features/Chat/Thread";
import { selectApiKey, selectConfig } from "../../features/Config/configSlice";
import { sendChatCommand } from "../../services/refact/chatCommands";
import type { ToolCall } from "../../services/refact/types";
import {
  Badge,
  Button,
  Dialog,
  FieldText,
  FieldTextarea,
  Icon,
  IconButton,
  Select,
  StatusDot,
} from "../ui";
import { TranscriptItem, type TranscriptStatus } from "./TranscriptItem";
import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../shared/useDelayedUnmount";
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

const EMPTY_ALERTS = { stuck: 0, failed: 0, approval: 0 };

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

function priorityBadgeTone(
  priority: string,
): React.ComponentProps<typeof Badge>["tone"] {
  switch (priority) {
    case "P0":
      return "danger";
    case "P1":
      return "warning";
    case "P2":
      return "accent";
    default:
      return "muted";
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
    case "queued":
    case "parked":
    case "cancelled":
    case "unknown":
      return styles.mutedValue;
  }
}

function stateStatus(
  state: AgentStatusState,
): React.ComponentProps<typeof StatusDot>["status"] {
  switch (state) {
    case "failed":
      return "error";
    case "done":
      return "success";
    case "stuck":
    case "paused":
      return "warning";
    case "running":
      return "running";
    case "queued":
    case "parked":
    case "cancelled":
    case "unknown":
      return "idle";
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
    case "queued":
      return "Queued";
    case "parked":
      return "Parked";
    case "stuck":
      return "Stuck";
    case "failed":
      return "Failed";
    case "done":
      return "Done";
    case "paused":
      return "Paused";
    case "cancelled":
      return "Cancelled";
    case "unknown":
      return "Unknown";
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
  if (!value) return <span className={styles.mutedValue}>{empty}</span>;
  return value;
}

function AgentRowCard({
  row,
  isExpanded,
  actionsDisabled,
  isSubmitting,
  onToggle,
  onPulse,
  onDiff,
  onSteer,
  onCancel,
}: {
  row: AgentStatusRow;
  isExpanded: boolean;
  actionsDisabled: boolean;
  isSubmitting: boolean;
  onToggle: (cardId: string) => void;
  onPulse: (row: AgentStatusRow) => void;
  onDiff: (row: AgentStatusRow) => void;
  onSteer: (row: AgentStatusRow) => void;
  onCancel: (row: AgentStatusRow) => void;
}) {
  const disabled = actionsDisabled || isSubmitting;
  const isVariant = /[/:]/u.test(row.cardId);
  const detailsId = React.useId();
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isExpanded,
    COLLAPSE_ANIMATION_MS,
  );
  const shouldRenderDetails = isExpanded || shouldRender;

  return (
    <article className={classNames(styles.agentCard, "rf-enter-rise")}>
      <div className={styles.agentCardHeader}>
        <div className={styles.agentCardMain}>
          <div className={styles.cardTitleRow}>
            <Badge tone={priorityBadgeTone(row.priority)}>{row.priority}</Badge>
            <a href={`#${row.cardId}`} className={styles.cardLink}>
              {row.cardId}
            </a>
            <span className={styles.cardTitle}>{row.title}</span>
          </div>
          <div className={styles.stateRow}>
            <StatusDot
              status={stateStatus(row.state)}
              pulse={row.state === "running"}
            />
            <span
              className={classNames(
                styles.stateText,
                stateClass(row.state),
                row.state === "running" && "rf-status-pulse",
              )}
            >
              {row.stateText}
            </span>
          </div>
        </div>
        <IconButton
          aria-controls={detailsId}
          aria-expanded={isExpanded}
          aria-label={`Toggle details ${row.cardId}`}
          icon={ClipboardList}
          size="sm"
          variant={isExpanded ? "soft" : "ghost"}
          onClick={() => onToggle(row.cardId)}
        />
      </div>

      <div className={styles.cardMetaGrid}>
        <div>
          <span className={styles.cellLabel}>Age</span>
          <span className={styles.cellValue}>{row.age}</span>
        </div>
        <div>
          <span className={styles.cellLabel}>Last tool</span>
          <span className={styles.cellValue}>{row.lastTool ?? "—"}</span>
        </div>
        <div>
          <span className={styles.cellLabel}>State</span>
          <span className={styles.cellValue}>{tabLabel(row.state)}</span>
        </div>
      </div>

      <div className={styles.actions}>
        <IconButton
          aria-label={`View pulse ${row.cardId}`}
          icon={Eye}
          size="sm"
          variant="ghost"
          disabled={disabled}
          onClick={() => onPulse(row)}
        />
        <IconButton
          aria-label={`View diff ${row.cardId}`}
          icon={FileDiff}
          size="sm"
          variant="ghost"
          disabled={disabled}
          onClick={() => onDiff(row)}
        />
        <IconButton
          aria-label={`Steer ${row.cardId}`}
          title={
            isVariant
              ? "Variant steering is unavailable"
              : "Request planner steering"
          }
          icon={Hand}
          size="sm"
          variant="ghost"
          disabled={disabled || isVariant}
          onClick={() => onSteer(row)}
        />
        <IconButton
          aria-label={`Cancel agent ${row.cardId}`}
          title={
            isVariant
              ? "Variant cancellation is unavailable"
              : "Request planner cancellation"
          }
          icon={CircleStop}
          size="sm"
          variant="danger"
          disabled={disabled || isVariant}
          onClick={() => onCancel(row)}
        />
      </div>

      {isVariant && (
        <div className={styles.emptyState}>
          Variant steering and cancellation are unavailable; inspection targets{" "}
          {row.cardId} only.
        </div>
      )}

      {shouldRenderDetails && (
        <div
          className={classNames("rf-expand-grid", styles.detailsGrid)}
          data-open={isAnimatingOpen}
          id={detailsId}
        >
          <div className={styles.detailsShell}>
            <div className={styles.details}>
              <div className={styles.detailBlock}>
                <span className={styles.detailLabel}>Last status update</span>
                <div className={styles.detailValue}>
                  {renderDetailValue(
                    row.lastStatusUpdate,
                    "Not included in compact output.",
                  )}
                </div>
              </div>
              <div className={styles.detailBlock}>
                <span className={styles.detailLabel}>Final report excerpt</span>
                <div className={styles.detailValue}>
                  {renderDetailValue(
                    row.finalReport ? truncateText(row.finalReport, 300) : null,
                    "No final report in this output.",
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </article>
  );
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
  const alertCount = alerts.stuck + alerts.failed + alerts.approval;
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
    <div className={styles.root}>
      {alertCount > 0 && (
        <div className={styles.stickyAlerts}>
          <div
            className={classNames(
              styles.alert,
              alerts.failed > 0 && styles.alertDanger,
            )}
          >
            <Icon
              icon={CircleAlert}
              size="sm"
              tone={alerts.failed > 0 ? "danger" : "warning"}
            />
            <span>
              {alerts.stuck} stuck, {alerts.failed} failed, {alerts.approval}{" "}
              needing approval
            </span>
          </div>
        </div>
      )}

      <div
        className={styles.tabsList}
        role="tablist"
        aria-label="Agent status filters"
      >
        {STATUS_TABS.map((item) => (
          <Button
            key={item}
            role="tab"
            aria-selected={tab === item}
            size="sm"
            variant={tab === item ? "soft" : "plain"}
            onClick={() => handleTabChange(item)}
          >
            {tabLabel(item)} {tabCount(report.rows, item)}
          </Button>
        ))}
      </div>

      <div className={styles.filters}>
        <label className={styles.filterGroup}>
          <span className={styles.filterLabel}>Priority</span>
          <Select value={priority} onValueChange={handlePriorityChange}>
            <Select.Trigger aria-label="Priority filter" />
            <Select.Content>
              <Select.Item value="all">All priorities</Select.Item>
              <Select.Item value="P0">P0</Select.Item>
              <Select.Item value="P1">P1</Select.Item>
              <Select.Item value="P2">P2</Select.Item>
            </Select.Content>
          </Select>
        </label>

        <label className={styles.filterGroup}>
          <span className={styles.filterLabel}>Age</span>
          <Select value={ageFilter} onValueChange={handleAgeChange}>
            <Select.Trigger aria-label="Age filter" />
            <Select.Content>
              <Select.Item value="all">Any age</Select.Item>
              <Select.Item value="15">15m+</Select.Item>
              <Select.Item value="60">1h+</Select.Item>
              <Select.Item value="240">4h+</Select.Item>
            </Select.Content>
          </Select>
        </label>
      </div>

      <div className={styles.emptyState}>
        Actions send requests to the planner chat, not direct tool executions.
      </div>

      <div className={classNames(styles.cards, "rf-stagger")} role="table">
        {visibleRows.map((row) => (
          <AgentRowCard
            key={row.cardId}
            row={row}
            isExpanded={expandedRows.has(row.cardId)}
            actionsDisabled={actionsDisabled || !onSubmitCommand}
            isSubmitting={isSubmitting}
            onToggle={toggleExpanded}
            onPulse={(selectedRow) => {
              void submitCommand(
                `View pulse ${selectedRow.cardId}`,
                formatAgentActionCommand("pulse", selectedRow.cardId),
              );
            }}
            onDiff={(selectedRow) => {
              void submitCommand(
                `View diff ${selectedRow.cardId}`,
                formatAgentActionCommand("diff", selectedRow.cardId),
              );
            }}
            onSteer={openSteerDialog}
            onCancel={openCancelDialog}
          />
        ))}
      </div>

      {visibleRows.length === 0 && (
        <div className={styles.emptyState}>
          {report.roster === "empty"
            ? "No agent rows returned in this response."
            : "No agents match the selected filters."}
        </div>
      )}

      <Dialog
        open={dialog !== null}
        onOpenChange={(open) => !open && closeDialog()}
      >
        <Dialog.Content className={styles.dialogContent}>
          {dialog?.kind === "queued" && (
            <>
              <Dialog.Title>{dialog.title}</Dialog.Title>
              <Dialog.Description>
                {isSubmitting
                  ? "Submitting a request to the planner chat queue."
                  : dialogError
                    ? "The request could not be submitted."
                    : "Request sent to the planner chat queue; execution is not confirmed."}
              </Dialog.Description>
              <div className={styles.commandPreview}>{dialog.command}</div>
            </>
          )}

          {dialog?.kind === "steer" && (
            <>
              <Dialog.Title>Steer {dialog.row.cardId}</Dialog.Title>
              <Dialog.Description>
                Ask the planner to send a steering message to this agent.
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
              <Dialog.Title>Cancel {dialog.row.cardId}</Dialog.Title>
              <Dialog.Description>
                Request cancellation from the planner and optionally edit the
                reason.
              </Dialog.Description>
              <FieldText
                aria-label="Cancel reason"
                value={cancelReason}
                onChange={setCancelReason}
              />
            </>
          )}

          {dialogError && (
            <div className={classNames(styles.alert, styles.alertDanger)}>
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
              {dialog?.kind === "queued" ? "Close" : "Cancel"}
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
                onClick={submitCancel}
                disabled={isSubmitting}
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

export const AgentStatusView: React.FC<AgentStatusViewProps> = ({
  toolCall,
}) => {
  const chatId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, chatId),
  );
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
    () => (content ? parseAgentStatusOutput(content) : null),
    [content],
  );

  const alerts = report
    ? mergeAgentAlerts(report.alerts, countAgentAlerts(report.rows))
    : EMPTY_ALERTS;
  const parsed = report !== null && report.roster !== "unparsed";
  const rowCount = parsed ? report.rows.length : null;
  const isRequestRunning = !maybeResult && (isStreaming || isWaiting);
  const failed = maybeResult?.tool_failed === true;
  const status: TranscriptStatus = failed
    ? "failed"
    : isRequestRunning
      ? "running"
      : !maybeResult
        ? "unknown"
        : alerts.failed > 0
          ? "failed"
          : alerts.approval > 0 || alerts.stuck > 0
            ? "needs_you"
            : "done";
  const statusLabel =
    !failed && alerts.failed > 0
      ? `${alerts.failed} failed`
      : alerts.approval > 0
        ? `${alerts.approval} need approval`
        : alerts.stuck > 0
          ? `${alerts.stuck} stuck`
          : undefined;
  const facts = parsed
    ? [
        { label: "Agents", value: String(rowCount) },
        ...(report.coverage
          ? [{ label: "Coverage", value: report.coverage }]
          : []),
        {
          label: "Alerts",
          value: `${alerts.stuck} stuck · ${alerts.failed} failed · ${alerts.approval} approval`,
        },
      ]
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
      data-testid="agent-status-view"
      icon={<Settings aria-hidden />}
      verb="Agents"
      target={rowCount !== null ? `${rowCount} cards` : undefined}
      status={status}
      statusLabel={statusLabel}
      summary={facts}
      output={
        parsed ? (
          <AgentStatusContent
            report={report}
            onSubmitCommand={handleSubmitCommand}
            actionsDisabled={!chatId || failed}
          />
        ) : undefined
      }
      error={failed ? content : undefined}
      raw={[
        { label: "Arguments", value: toolCall.function.arguments },
        ...(content !== null ? [{ label: "Response", value: content }] : []),
      ]}
      openKey={toolCall.id ? `agent_status:${toolCall.id}` : undefined}
      defaultOpen={status === "needs_you"}
    />
  );
};

export default AgentStatusView;
