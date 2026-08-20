import React, { useCallback, useId, useMemo, useState } from "react";
import classNames from "classnames";
import { Bot, Telescope } from "lucide-react";
import { Badge, Button, Icon } from "../ui";
import { humanizeIdentifier } from "../../utils/displayNames";
import styles from "./BackgroundAgentCard.module.css";
import type { BackgroundAgentSummary } from "../../services/refact/types";

export interface BackgroundAgentCardProps {
  agent: BackgroundAgentSummary;
  onOpenTrajectory?: (childChatId: string) => void;
}

type Tone = "accent" | "success" | "danger" | "warning" | "muted";

const TERMINAL_STATUSES = new Set<BackgroundAgentSummary["status"]>([
  "completed",
  "failed",
  "cancelled",
  "interrupted",
]);

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

/**
 * Relative last-activity label. Mirrors the chat-land helpers
 * (PlanBanner/buddyUtils): never surface a raw ISO timestamp in chrome.
 */
function formatRelativeActivity(value: string | null): string | null {
  if (!value) return null;
  const time = Date.parse(value);
  if (!Number.isFinite(time)) return null;
  const diff = Math.max(0, Date.now() - time);
  if (diff < MINUTE_MS) return "just now";
  if (diff < HOUR_MS) return `${Math.floor(diff / MINUTE_MS)}m ago`;
  if (diff < DAY_MS) return `${Math.floor(diff / HOUR_MS)}h ago`;
  return `${Math.floor(diff / DAY_MS)}d ago`;
}

/** Longest shared directory prefix across paths (mirrors AnalysisReport). */
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

/** Same contract as ToolCard/AnalysisReport's shortenPath. */
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

export const BackgroundAgentCard = ({
  agent,
  onOpenTrajectory,
}: BackgroundAgentCardProps) => {
  const [filesOpen, setFilesOpen] = useState(false);
  const panelId = useId();

  const isRunning = agent.status === "running";
  const isTerminal = TERMINAL_STATUSES.has(agent.status);
  const tone = statusTone(agent.status);

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

  const handleToggleFiles = useCallback(() => {
    setFilesOpen((open) => !open);
  }, []);

  const handleOpenTrajectory = useCallback(() => {
    if (agent.child_chat_id && onOpenTrajectory) {
      onOpenTrajectory(agent.child_chat_id);
    }
  }, [agent.child_chat_id, onOpenTrajectory]);

  return (
    <div className={styles.card} data-testid="background-agent-card">
      <div className={styles.header}>
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
        <Badge
          tone={tone}
          size="xs"
          variant="soft"
          className={styles.statusChip}
          data-testid="background-agent-status"
        >
          <span
            aria-hidden="true"
            className={classNames(
              styles.statusDot,
              isRunning && styles.statusDotPulse,
            )}
          />
          {humanizeIdentifier(agent.status)}
        </Badge>
        {relativeActivity && (
          <span className={styles.time}>{relativeActivity}</span>
        )}
        <ShortIdChip agentId={agent.agent_id} />
      </div>

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

      {files.list.length > 0 && (
        <div>
          <button
            type="button"
            className={styles.filesToggle}
            aria-expanded={filesOpen}
            aria-controls={panelId}
            onClick={handleToggleFiles}
          >
            {files.list.length} {files.label}
          </button>
          <div
            id={panelId}
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
