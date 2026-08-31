import React, { useCallback, useId, useMemo, useState } from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import classNames from "classnames";
import { Bot, Telescope } from "lucide-react";
import { Badge, Button, Icon } from "../ui";
import { Chevron } from "../Collapsible";
import { humanizeIdentifier } from "../../utils/displayNames";
import { formatTokenCount } from "../../features/StatsDashboard/utils/formatters";
import styles from "./BackgroundAgentCard.module.css";
import type {
  AgentQuestion,
  BackgroundAgentMergeStatus,
  BackgroundAgentSummary,
} from "../../services/refact/types";

export interface BackgroundAgentCardProps {
  agent: BackgroundAgentSummary;
  compactDefault?: boolean;
  onOpenTrajectory?: (childChatId: string) => void;
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

function formatUsage(agent: BackgroundAgentSummary): string | null {
  const hasTokens = (agent.tokens_used ?? 0) > 0;
  const hasCost = agent.cost_usd !== null && agent.cost_usd !== undefined;
  if (!hasTokens && !hasCost) return null;
  const parts: string[] = [];
  if (hasTokens) {
    parts.push(
      `${formatTokenCount(agent.tokens_used ?? 0).replace("K", "k")} tok`,
    );
  }
  if (hasCost) parts.push(`$${(agent.cost_usd ?? 0).toFixed(2)}`);
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
  const isRunning = agent.status === "running";
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
      {agent.current_tool && (
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
        {agent.current_tool && (
          <span className={styles.detailNow}>now: {agent.current_tool}</span>
        )}
        {relativeActivity && (
          <span className={styles.time}>{relativeActivity}</span>
        )}
        <ShortIdChip agentId={agent.agent_id} />
      </div>

      {(agent.goal_summary ?? agent.plan_present ?? agent.worktree_branch) && (
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

export const BackgroundAgentCard: React.FC<BackgroundAgentCardProps> = ({
  agent,
  compactDefault = true,
  onOpenTrajectory,
}) => {
  const [expanded, setExpanded] = useState(!compactDefault);
  const [filesOpen, setFilesOpen] = useState(false);
  const filesPanelId = useId();

  const handleToggleFiles = useCallback(() => {
    setFilesOpen((open) => !open);
  }, []);

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
            <CompactRow agent={agent} expanded={expanded} />
          </button>
        </Collapsible.Trigger>
        <Collapsible.Content className={styles.content}>
          <ExpandedDetail
            agent={agent}
            filesOpen={filesOpen}
            filesPanelId={filesPanelId}
            onToggleFiles={handleToggleFiles}
            onOpenTrajectory={onOpenTrajectory}
          />
        </Collapsible.Content>
      </div>
    </Collapsible.Root>
  );
};
