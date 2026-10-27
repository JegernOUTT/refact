import React, { useCallback, useId, useMemo, useState } from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import classNames from "classnames";
import { Bot, MessageCircleQuestion, Telescope } from "lucide-react";
import { Badge, Button, Icon } from "../ui";
import { Chevron } from "../Collapsible";
import { humanizeIdentifier } from "../../utils/displayNames";
import { formatTokenCount } from "../../features/StatsDashboard/utils/formatters";
import styles from "./BackgroundAgentCard.module.css";
import type {
  AgentQuestion,
  BackgroundAgentMergeStatus,
  BackgroundAgentSummary,
  DeliverableStatus,
  HandoffGap,
  ToolCall,
} from "../../services/refact/types";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectBackgroundAgent } from "../../features/Chat/Thread/selectors";
import { ToolCard } from "../ui/ToolCard/ToolCard";
import { useStoredOpen } from "../ChatContent/useStoredOpen";
import {
  useChatScrollAnchor,
  usePrepareChatScrollAnchor,
} from "../ChatContent/useChatScrollAnchor";
import { CompactToolDetails } from "../Tools/AgenticTool/CompactToolDetails";
import { compactText } from "../Tools/AgenticTool/compactPresentation";
import { ToolCallTooltip } from "../ChatContent/ToolCard/ToolCallTooltip";

export interface BackgroundAgentCardProps {
  agent: BackgroundAgentSummary;
  threadId?: string;
  agentId?: string;
  compactDefault?: boolean;
  presentation?: "panel" | "inline";
  statusKnown?: boolean;
  launchFailed?: boolean;
  toolCallId?: string;
  toolCall?: ToolCall;
  children?: React.ReactNode;
  onOpenTrajectory?: (childChatId: string) => void;
  onOpenAgentTrajectory?: (
    agent: BackgroundAgentSummary,
    childChatId: string,
  ) => void;
}

type Tone = "accent" | "success" | "danger" | "warning" | "muted";

const TERMINAL_STATUSES = new Set<BackgroundAgentSummary["status"]>([
  "completed",
  "failed",
  "cancelled",
  "interrupted",
]);

const MERGE_BADGES: Record<
  BackgroundAgentMergeStatus,
  { symbol: string; tone: Tone }
> = {
  pending: { symbol: "○", tone: "muted" },
  merged: { symbol: "✅", tone: "success" },
  conflict: { symbol: "⚠", tone: "warning" },
  skipped: { symbol: "○", tone: "muted" },
  failed: { symbol: "✖", tone: "danger" },
};

function statusTone(status: BackgroundAgentSummary["status"]): Tone {
  switch (status) {
    case "running":
      return "accent";
    case "completed":
      return "success";
    case "failed":
      return "danger";
    case "queued":
    case "waiting_for_approval":
      return "warning";
    default:
      return "muted";
  }
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

function formatRelativeActivity(
  value: string | null | undefined,
): string | null {
  if (!value) return null;
  const time = Date.parse(value);
  if (!Number.isFinite(time)) return null;
  const diff = Math.max(0, Date.now() - time);
  if (diff < MINUTE_MS) return "just now";
  if (diff < HOUR_MS) return `${Math.floor(diff / MINUTE_MS)}m ago`;
  if (diff < DAY_MS) return `${Math.floor(diff / HOUR_MS)}h ago`;
  return `${Math.floor(diff / DAY_MS)}d ago`;
}

function commonPathPrefix(paths: string[]): string | null {
  if (paths.length < 2) return null;
  const split = paths.map((path) => path.split("/"));
  const first = split[0];
  let shared = 0;
  for (let index = 0; index < first.length - 1; index++) {
    if (split.every((segments) => segments[index] === first[index])) shared++;
    else break;
  }
  return shared < 1 ? null : `${first.slice(0, shared).join("/")}/`;
}

function shortenPath(path: string, prefix: string | null): string {
  return prefix && path.startsWith(prefix) ? path.slice(prefix.length) : path;
}

function shortId(agentId: string): string {
  return agentId.slice(-8);
}

function parseDiffCounts(
  summary: string | null,
): { adds: number; dels: number } | null {
  if (!summary) return null;
  const adds = /\+\s*(\d+)/.exec(summary);
  const dels = /[-−]\s*(\d+)/.exec(summary);
  if (!adds && !dels) return null;
  return {
    adds: adds ? Number(adds[1]) : 0,
    dels: dels ? Number(dels[1]) : 0,
  };
}

function pendingQuestions(agent: BackgroundAgentSummary): number {
  if (agent.pending_questions !== undefined) return agent.pending_questions;
  return (agent.questions ?? []).filter((question) => !question.answer).length;
}

function deliverableTitle(status: DeliverableStatus): string {
  const parts = [`${status.check} check`];
  if (status.missing_sections.length > 0) {
    parts.push(`missing sections: ${status.missing_sections.join(", ")}`);
  }
  if (status.schema_error) {
    parts.push(`output schema: ${status.schema_error}`);
  }
  if (status.evidence_missing) {
    parts.push("evidence section missing; claims are unverified");
  }
  return parts.join(" · ");
}

function handoffItem(gap: HandoffGap): string {
  const detail = gap.detail.trim() || gap.category.replace(/_/g, " ");
  const item = gap.path ? `${detail} (${gap.path})` : detail;
  const characters = Array.from(item);
  return characters.length > 300
    ? `${characters.slice(0, 299).join("")}…`
    : item;
}

const HandoffSection: React.FC<{ agent: BackgroundAgentSummary }> = ({
  agent,
}) => {
  const gaps = agent.handoff?.gaps ?? [];
  const discovered = gaps.filter(
    (gap) =>
      gap.category !== "missing_decision" &&
      gap.category !== "ambiguous_requirement",
  );
  const guessed = gaps.filter(
    (gap) =>
      gap.category === "missing_decision" ||
      gap.category === "ambiguous_requirement",
  );
  const groups = [
    { label: "Had to discover", items: discovered },
    { label: "Had to guess", items: guessed },
  ];

  return (
    <section className={styles.handoffSection} aria-label="Handoff">
      <h3 className={styles.detailLabel}>Handoff</h3>
      {groups.every(({ items }) => items.length === 0) ? (
        <p className={styles.handoffEmpty}>Nothing declared</p>
      ) : (
        groups.map(({ label, items }) =>
          items.length === 0 ? null : (
            <div className={styles.handoffListGroup} key={label}>
              <h4 className={styles.handoffListLabel}>{label}</h4>
              <ul className={styles.handoffList}>
                {items.slice(0, 10).map((gap, index) => (
                  <li
                    className={styles.handoffItem}
                    key={`${gap.category}-${index}`}
                  >
                    {handoffItem(gap)}
                  </li>
                ))}
              </ul>
              {items.length > 10 && (
                <span className={styles.handoffMore}>
                  +{items.length - 10} more
                </span>
              )}
            </div>
          ),
        )
      )}
    </section>
  );
};

function formatUsage(agent: BackgroundAgentSummary): string | null {
  const hasTokens = (agent.tokens_used ?? 0) > 0;
  const hasCost =
    typeof agent.cost_usd === "number" &&
    Number.isFinite(agent.cost_usd) &&
    agent.cost_usd > 0;
  if (!hasTokens && !hasCost) return null;
  const parts: string[] = [];
  if (hasTokens) {
    parts.push(
      `${formatTokenCount(agent.tokens_used ?? 0).replace("K", "k")} tok`,
    );
  }
  if (hasCost && typeof agent.cost_usd === "number") {
    parts.push(`$${agent.cost_usd.toFixed(2)}`);
  }
  return parts.join(" · ");
}

const ShortIdChip: React.FC<{ agentId: string }> = ({ agentId }) => {
  const handleCopy = useCallback(() => {
    void navigator.clipboard.writeText(agentId);
  }, [agentId]);

  return (
    <button
      type="button"
      className={styles.shortId}
      title={agentId}
      aria-label="Copy agent id"
      onClick={handleCopy}
    >
      {shortId(agentId)}
    </button>
  );
};

const CompactRow: React.FC<{
  agent: BackgroundAgentSummary;
  expanded: boolean;
}> = ({ agent, expanded }) => {
  const isTerminal = TERMINAL_STATUSES.has(agent.status);
  const isRunning = !isTerminal && agent.status === "running";
  const modelLabel = agent.model_type ?? agent.model;
  const modelTitle = agent.model ?? agent.model_type ?? undefined;
  const usage = formatUsage(agent);
  const questions = pendingQuestions(agent);
  const merge = agent.merge_status
    ? MERGE_BADGES[agent.merge_status]
    : undefined;
  const status = humanizeIdentifier(agent.status);

  return (
    <div
      className={styles.compactRow}
      data-testid="background-agent-compact-row"
    >
      <span
        aria-label={`Background agent status: ${status}`}
        className={classNames(
          styles.statusDot,
          styles[`statusDot${statusTone(agent.status)}`],
          isRunning && styles.statusDotPulse,
        )}
        data-testid="background-agent-status-dot"
        title={status}
      />
      <span
        className={classNames(
          styles.kindTile,
          agent.kind === "delegate" && styles.kindTileDelegate,
        )}
        data-testid={`background-agent-kind-${agent.kind}`}
        title={humanizeIdentifier(agent.kind)}
      >
        <Icon
          icon={agent.kind === "delegate" ? Bot : Telescope}
          size="sm"
          tone={agent.kind === "delegate" ? "accent" : "muted"}
        />
      </span>
      <span className={styles.title} title={agent.title}>
        {agent.title}
      </span>
      {modelLabel && (
        <Badge
          tone="muted"
          size="xs"
          variant="soft"
          className={styles.modelChip}
          data-testid="background-agent-model"
          title={modelTitle}
        >
          {modelLabel}
        </Badge>
      )}
      {!isTerminal && agent.current_tool && (
        <span
          className={styles.currentTool}
          data-testid="background-agent-current-tool"
          title={agent.current_tool}
        >
          <span className={styles.currentToolLabel}>now: </span>
          <span className={styles.currentToolValue} key={agent.current_tool}>
            {agent.current_tool}
          </span>
        </span>
      )}
      {usage && (
        <span
          className={styles.usage}
          data-testid="background-agent-usage"
          title="Background agent token usage and cost"
        >
          {usage}
        </span>
      )}
      {questions > 0 && (
        <Badge
          tone="warning"
          size="xs"
          variant="soft"
          className={styles.questionsBadge}
          data-testid="background-agent-questions"
          title={`${questions} pending question${questions === 1 ? "" : "s"}`}
        >
          ❓{questions}
        </Badge>
      )}
      {merge && agent.merge_status && (
        <Badge
          tone={merge.tone}
          size="xs"
          variant="soft"
          className={styles.mergeBadge}
          data-testid="background-agent-merge"
          title={
            agent.conflict_summary ?? humanizeIdentifier(agent.merge_status)
          }
        >
          {merge.symbol} {humanizeIdentifier(agent.merge_status)}
        </Badge>
      )}
      <Chevron className={styles.chevron} open={expanded} />
    </div>
  );
};

const QuestionRow: React.FC<{ question: AgentQuestion }> = ({ question }) => {
  const askedAt = formatRelativeActivity(question.asked_at);
  const answeredAt = formatRelativeActivity(question.answered_at);

  return (
    <li className={styles.questionRow}>
      <span className={styles.questionText}>Q: {question.text}</span>
      <span className={styles.questionAnswer}>
        {question.answer ? `A: ${question.answer}` : "Awaiting reply"}
      </span>
      {(askedAt ?? answeredAt) && (
        <span className={styles.questionTime}>
          {answeredAt ? `answered ${answeredAt}` : `asked ${askedAt}`}
        </span>
      )}
    </li>
  );
};

const ExpandedDetail: React.FC<{
  agent: BackgroundAgentSummary;
  filesOpen: boolean;
  filesPanelId: string;
  onToggleFiles: () => void;
  onOpenTrajectory?: (childChatId: string) => void;
}> = ({ agent, filesOpen, filesPanelId, onToggleFiles, onOpenTrajectory }) => {
  const isRunning = agent.status === "running";
  const isTerminal = TERMINAL_STATUSES.has(agent.status);
  const files = useMemo(() => {
    const preferEdited = agent.edited_files.length > 0 && isTerminal;
    return preferEdited
      ? { list: agent.edited_files, label: "edited files" }
      : agent.target_files.length > 0
        ? { list: agent.target_files, label: "target files" }
        : { list: agent.edited_files, label: "edited files" };
  }, [agent.edited_files, agent.target_files, isTerminal]);
  const prefix = useMemo(() => commonPathPrefix(files.list), [files.list]);
  const relativeActivity = formatRelativeActivity(agent.last_activity);
  const diffCounts = parseDiffCounts(agent.diff_summary);
  const usage = formatUsage(agent);

  const handleOpenTrajectory = useCallback(() => {
    if (agent.child_chat_id && onOpenTrajectory) {
      onOpenTrajectory(agent.child_chat_id);
    }
  }, [agent.child_chat_id, onOpenTrajectory]);

  return (
    <div
      className={styles.expandedDetail}
      data-testid="background-agent-expanded-detail"
    >
      <div className={styles.detailMeta}>
        {!isTerminal && agent.current_tool && (
          <span className={styles.detailNow}>now: {agent.current_tool}</span>
        )}
        {relativeActivity && (
          <span className={styles.time}>{relativeActivity}</span>
        )}
        <ShortIdChip agentId={agent.agent_id} />
      </div>

      {(Boolean(agent.goal_summary) ||
        Boolean(agent.plan_present) ||
        Boolean(agent.worktree_branch)) && (
        <div className={styles.chipRow}>
          {agent.goal_summary && (
            <Badge
              tone="accent"
              size="xs"
              variant="soft"
              title={agent.goal_summary}
            >
              🎯 Goal
            </Badge>
          )}
          {agent.plan_present && (
            <Badge tone="muted" size="xs" variant="soft" title="Plan available">
              📋 Plan
            </Badge>
          )}
          {agent.worktree_branch && (
            <Badge
              tone="muted"
              size="xs"
              variant="soft"
              className={styles.branchChip}
              title={agent.worktree_branch}
            >
              {agent.worktree_branch}
            </Badge>
          )}
        </div>
      )}

      {isRunning ? (
        <>
          <div
            className={styles.progressTrack}
            role="progressbar"
            aria-label="Background agent activity"
            data-testid="background-agent-progress"
          >
            <div className={styles.progressBar} />
          </div>
          <span className={styles.stepText}>
            step {agent.step_count}
            {agent.progress ? ` · ${agent.progress}` : ""}
          </span>
        </>
      ) : (
        <div className={styles.resultRow}>
          {agent.edited_files.length > 0 && (
            <Badge tone="muted" size="xs" variant="soft">
              {agent.edited_files.length} edited
            </Badge>
          )}
          {diffCounts && (
            <Badge tone="accent" size="xs" variant="soft">
              +{diffCounts.adds} −{diffCounts.dels}
            </Badge>
          )}
          {agent.conflict_summary && (
            <Badge
              tone="warning"
              size="xs"
              variant="soft"
              title={agent.conflict_summary}
            >
              Conflicts
            </Badge>
          )}
          {agent.deliverable_status ? (
            <Badge
              tone={agent.deliverable_status.met ? "accent" : "warning"}
              size="xs"
              variant="soft"
              data-testid="background-agent-deliverable"
              title={deliverableTitle(agent.deliverable_status)}
            >
              {agent.deliverable_status.met
                ? "Deliverable met"
                : "Deliverable unmet"}
            </Badge>
          ) : (
            agent.deliverable && (
              <Badge
                tone="muted"
                size="xs"
                variant="soft"
                data-testid="background-agent-deliverable"
                title="A deliverable contract was declared but no completed final message was evaluated"
              >
                Deliverable not evaluated
              </Badge>
            )
          )}
          {agent.error && (
            <Badge tone="danger" size="xs" variant="soft" title={agent.error}>
              {humanizeIdentifier("failed")}
            </Badge>
          )}
          {(agent.result_summary ?? agent.error) && (
            <span className={styles.resultText}>
              {agent.result_summary ?? agent.error}
            </span>
          )}
        </div>
      )}

      {isTerminal && <HandoffSection agent={agent} />}

      {usage && <span className={styles.detailUsage}>{usage}</span>}

      {files.list.length > 0 && (
        <div>
          <button
            type="button"
            className={styles.filesToggle}
            aria-expanded={filesOpen}
            aria-controls={filesPanelId}
            onClick={onToggleFiles}
          >
            {files.list.length} {files.label}
          </button>
          <div
            id={filesPanelId}
            className={classNames(
              styles.filesPanel,
              filesOpen && styles.filesPanelOpen,
            )}
          >
            <div className={styles.filesPanelInner}>
              {filesOpen && (
                <>
                  <ul className={styles.fileList}>
                    {files.list.map((file) => (
                      <li className={styles.fileItem} key={file} title={file}>
                        {shortenPath(file, prefix)}
                      </li>
                    ))}
                  </ul>
                  {prefix && (
                    <div className={styles.prefixHint}>…in {prefix}</div>
                  )}
                </>
              )}
            </div>
          </div>
        </div>
      )}

      {(agent.questions ?? []).length > 0 && (
        <section
          className={styles.questionsSection}
          aria-label="Agent questions and answers"
        >
          <span className={styles.detailLabel}>Q&A</span>
          <ul className={styles.questionList}>
            {(agent.questions ?? []).map((question) => (
              <QuestionRow key={question.id} question={question} />
            ))}
          </ul>
        </section>
      )}

      {agent.child_chat_id && onOpenTrajectory && (
        <div className={styles.footer}>
          <Button size="sm" variant="soft" onClick={handleOpenTrajectory}>
            Open trajectory
          </Button>
        </div>
      )}
    </div>
  );
};

const InlineBackgroundAgent: React.FC<{
  agent: BackgroundAgentSummary;
  statusKnown: boolean;
  launchFailed: boolean;
  toolCallId?: string;
  onOpenTrajectory?: (childChatId: string) => void;
  children?: React.ReactNode;
}> = ({
  agent,
  statusKnown,
  launchFailed,
  toolCallId,
  onOpenTrajectory,
  children,
}) => {
  const storeKey = toolCallId
    ? `tc:${toolCallId}`
    : `background-agent:${agent.agent_id}`;
  const [expanded, , setExpanded] = useStoredOpen(storeKey, false);
  const [filesOpen, toggleFiles] = useStoredOpen(`${storeKey}:files`, false);
  const filesPanelId = useId();
  const preserveScrollAnchor = useChatScrollAnchor();
  const prepareScrollAnchor = usePrepareChatScrollAnchor();
  const status =
    statusKnown &&
    (TERMINAL_STATUSES.has(agent.status) ||
      agent.status === "running" ||
      agent.status === "queued" ||
      agent.status === "waiting_for_approval")
      ? agent.status
      : null;
  const stateLabel =
    status === "waiting_for_approval"
      ? "Approval needed"
      : status
        ? humanizeIdentifier(status)
        : "Status unknown";
  const questions = pendingQuestions(agent);
  const tone = launchFailed
    ? "danger"
    : questions > 0
      ? "warning"
      : status
        ? statusTone(status)
        : "muted";
  const headline =
    compactText(
      agent.title.replace(/^\s*(?:subagent|delegate)\s*(?::\s*|$)/iu, ""),
      160,
    ) ?? "Background work";
  const facts = [
    { label: "Error", value: compactText(agent.error) },
    { label: "Reported result", value: compactText(agent.result_summary) },
    {
      label: "Activity",
      value: compactText(agent.progress ?? agent.current_tool),
    },
    { label: "Goal", value: compactText(agent.goal_summary) },
  ].flatMap(({ label, value }) => (value === null ? [] : [{ label, value }]));

  return (
    <ToolCard
      data-testid="background-agent-inline"
      density="compact"
      status={launchFailed ? "error" : "idle"}
      open={expanded}
      onOpenChange={(open) => preserveScrollAnchor(() => setExpanded(open))}
      onPointerDownCapture={prepareScrollAnchor}
      onMouseDownCapture={prepareScrollAnchor}
      onKeyDownCapture={prepareScrollAnchor}
      title={
        <span className={styles.inlineRow}>
          <Icon icon={Telescope} tone={tone} />
          <span className={styles.inlineTitle} title={agent.title}>
            {headline}
          </span>
          <span
            className={styles.inlineState}
            data-tone={
              launchFailed ? "danger" : status ? statusTone(status) : "muted"
            }
            data-testid="background-agent-inline-status"
            role={launchFailed ? "alert" : "status"}
          >
            {launchFailed ? `Request failed · ${stateLabel}` : stateLabel}
          </span>
          {questions > 0 && (
            <span
              className={styles.inlineAttention}
              aria-label={`${questions} pending ${
                questions === 1 ? "question" : "questions"
              }`}
              title={`${questions} pending ${
                questions === 1 ? "question" : "questions"
              }`}
            >
              <Icon icon={MessageCircleQuestion} size="sm" tone="warning" />
              {questions}
            </span>
          )}
        </span>
      }
    >
      <CompactToolDetails
        facts={facts}
        navigation={
          agent.child_chat_id && onOpenTrajectory ? (
            <Button
              size="sm"
              variant="soft"
              onClick={() => {
                if (agent.child_chat_id) onOpenTrajectory(agent.child_chat_id);
              }}
            >
              Open trajectory
            </Button>
          ) : undefined
        }
      >
        <div className={styles.inlineDetailMeta}>
          <span>{agent.title}</span>
          <span>{humanizeIdentifier(agent.kind)}</span>
          <span>{stateLabel}</span>
          {agent.model_type && <span>{agent.model_type}</span>}
          {agent.model && <span>{agent.model}</span>}
          {agent.merge_status && (
            <span>Local merge: {humanizeIdentifier(agent.merge_status)}</span>
          )}
        </div>
        <ExpandedDetail
          agent={agent}
          filesOpen={filesOpen}
          filesPanelId={filesPanelId}
          onToggleFiles={toggleFiles}
        />
        {children}
      </CompactToolDetails>
    </ToolCard>
  );
};

const _BackgroundAgentCard: React.FC<BackgroundAgentCardProps> = ({
  agent,
  threadId,
  agentId,
  compactDefault = true,
  presentation = "panel",
  statusKnown = true,
  launchFailed = false,
  toolCallId,
  toolCall,
  children,
  onOpenTrajectory,
  onOpenAgentTrajectory,
}) => {
  const subscribedAgent = useAppSelector((state) =>
    selectBackgroundAgent(state, threadId ?? "", agentId ?? agent.agent_id),
  );
  const displayedAgent = subscribedAgent ?? agent;
  const [expanded, setExpanded] = useState(!compactDefault);
  const [filesOpen, setFilesOpen] = useState(false);
  const filesPanelId = useId();

  const handleToggleFiles = useCallback(() => {
    setFilesOpen((open) => !open);
  }, []);
  const handleOpenTrajectory = useCallback(
    (childChatId: string) => {
      if (onOpenAgentTrajectory) {
        onOpenAgentTrajectory(displayedAgent, childChatId);
      } else {
        onOpenTrajectory?.(childChatId);
      }
    },
    [displayedAgent, onOpenAgentTrajectory, onOpenTrajectory],
  );

  if (presentation === "inline") {
    const inline = (
      <InlineBackgroundAgent
        agent={displayedAgent}
        statusKnown={Boolean(subscribedAgent) || statusKnown}
        launchFailed={launchFailed}
        toolCallId={toolCallId ?? toolCall?.id}
        onOpenTrajectory={
          onOpenTrajectory ?? onOpenAgentTrajectory
            ? handleOpenTrajectory
            : undefined
        }
      >
        {children}
      </InlineBackgroundAgent>
    );
    return toolCall ? (
      <ToolCallTooltip toolCall={toolCall}>{inline}</ToolCallTooltip>
    ) : (
      inline
    );
  }

  return (
    <Collapsible.Root open={expanded} onOpenChange={setExpanded}>
      <div
        className={classNames(styles.card, expanded && styles.cardExpanded)}
        data-expanded={expanded}
        data-testid="background-agent-card"
      >
        <Collapsible.Trigger asChild>
          <button
            type="button"
            className={styles.expandTrigger}
            aria-label={
              expanded
                ? "Collapse background agent details"
                : "Expand background agent details"
            }
          >
            <CompactRow agent={displayedAgent} expanded={expanded} />
          </button>
        </Collapsible.Trigger>
        <Collapsible.Content className={styles.content}>
          <ExpandedDetail
            agent={displayedAgent}
            filesOpen={filesOpen}
            filesPanelId={filesPanelId}
            onToggleFiles={handleToggleFiles}
            onOpenTrajectory={handleOpenTrajectory}
          />
        </Collapsible.Content>
      </div>
    </Collapsible.Root>
  );
};

export const BackgroundAgentCard = React.memo(_BackgroundAgentCard);
