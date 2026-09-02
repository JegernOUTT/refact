import classNames from "classnames";
import {
  Check,
  ChevronRight,
  CircleStop,
  Copy,
  MessageSquare,
  Send,
} from "lucide-react";
import { useState } from "react";

import {
  Badge,
  Button,
  IconButton,
  Popover,
  StatusDot,
  Tooltip,
} from "../../components/ui";
import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../../components/shared/useDelayedUnmount";
import { useCopyToClipboard } from "../../hooks";
import { useInternalLinkHandler } from "../../contexts/internalLinkUtils";
import {
  useCancelBackgroundAgentMutation,
  useMessageBackgroundAgentMutation,
  type BackgroundAgentSummary,
} from "../../services/refact";
import type { AgentTreeNode as AgentTreeNodeModel } from "../Chat/Thread";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { setError } from "../Errors/errorsSlice";
import { setInformation } from "../Errors/informationSlice";
import styles from "./AgentsPanel.module.css";

export type AgentTreeNodeProps = {
  chatId: string;
  node: AgentTreeNodeModel;
  depth?: number;
  onNavigate?: (chatId: string) => void;
};

const AGENT_TITLE_PREFIX = /^\s*(?:subagent|delegate)\s*:\s*/iu;
const MARKDOWN_EMPHASIS = /(\*\*|__|`)/gu;

function isTerminal(agent: BackgroundAgentSummary): boolean {
  return ["completed", "failed", "cancelled", "interrupted"].includes(
    agent.status,
  );
}

function statusFor(agent: BackgroundAgentSummary) {
  if (agent.status === "running") return "running" as const;
  if (agent.status === "completed") return "success" as const;
  if (agent.status === "failed") return "error" as const;
  if (agent.status === "waiting_for_approval") return "warning" as const;
  return "idle" as const;
}

function displayAgentTitle(agent: BackgroundAgentSummary): string {
  const raw = agent.title || agent.agent_id;
  const stripped = raw
    .replace(AGENT_TITLE_PREFIX, "")
    .replace(MARKDOWN_EMPHASIS, "")
    .trim();
  return stripped || raw;
}

function formatTokens(tokens: number | undefined): string {
  if (!tokens) return "—";
  if (tokens < 1000) return `${tokens}`;
  if (tokens < 1_000_000) {
    return `${(tokens / 1000).toFixed(tokens >= 10_000 ? 0 : 1)}k`;
  }
  return `${(tokens / 1_000_000).toFixed(1)}M`;
}

function formatCost(cost: number | null | undefined): string | null {
  if (
    cost === null ||
    cost === undefined ||
    !Number.isFinite(cost) ||
    cost <= 0
  ) {
    return null;
  }
  return `$${cost.toFixed(cost < 0.01 ? 4 : 2)}`;
}

function detailFiles(agent: BackgroundAgentSummary): string[] {
  return [...new Set([...agent.edited_files, ...agent.target_files])].slice(
    0,
    4,
  );
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message;
  return "Agent action failed.";
}

export function AgentTreeNode({
  chatId,
  node,
  depth = 0,
  onNavigate,
}: AgentTreeNodeProps) {
  const { agent, children } = node;
  const dispatch = useAppDispatch();
  const copyToClipboard = useCopyToClipboard();
  const internalLinks = useInternalLinkHandler();
  const [expanded, setExpanded] = useState(false);
  const [messageOpen, setMessageOpen] = useState(false);
  const [message, setMessage] = useState("");
  const [copied, setCopied] = useState<"id" | "branch" | null>(null);
  const [cancelAgent, cancelState] = useCancelBackgroundAgentMutation();
  const [messageAgent, messageState] = useMessageBackgroundAgentMutation();
  const terminal = isTerminal(agent);
  const hasChildren = children.length > 0;
  const cost = formatCost(agent.cost_usd);
  const files = detailFiles(agent);
  const questions = agent.questions ?? [];
  const title = displayAgentTitle(agent);
  const rawTitle = agent.title || agent.agent_id;
  const showMessageForm = messageOpen && !terminal;

  const expandedMotion = useDelayedUnmount(expanded, COLLAPSE_ANIMATION_MS);
  const messageMotion = useDelayedUnmount(
    showMessageForm,
    COLLAPSE_ANIMATION_MS,
  );
  const renderDetails = expanded || expandedMotion.shouldRender;
  const renderMessageForm = showMessageForm || messageMotion.shouldRender;

  const navigate = () => {
    if (!agent.child_chat_id) return;
    if (onNavigate) {
      onNavigate(agent.child_chat_id);
      return;
    }
    internalLinks?.handleInternalLink(`refact://chat/${agent.child_chat_id}`);
  };

  const handleCopy = (target: "id" | "branch", value: string) => {
    copyToClipboard(value);
    setCopied(target);
    window.setTimeout(() => setCopied(null), 1600);
    dispatch(
      setInformation(`${target === "id" ? "Agent ID" : "Branch"} copied.`),
    );
  };

  const handleCancel = async () => {
    try {
      await cancelAgent({
        agentId: agent.agent_id,
        chatId,
        subtree: true,
      }).unwrap();
      dispatch(
        setInformation("Cancellation requested for this agent subtree."),
      );
    } catch (error) {
      dispatch(setError(errorText(error)));
    }
  };

  const handleMessage = async () => {
    const text = message.trim();
    if (!text) return;
    try {
      await messageAgent({ agentId: agent.agent_id, chatId, text }).unwrap();
      setMessage("");
      setMessageOpen(false);
      dispatch(setInformation("Message sent to agent."));
    } catch (error) {
      dispatch(setError(errorText(error)));
    }
  };

  return (
    <li className={classNames(styles.node, "rf-enter-rise")} data-depth={depth}>
      <div
        className={styles.nodeRow}
        data-testid={`agent-row-${agent.agent_id}`}
        onClick={navigate}
      >
        <div className={styles.titleLine}>
          <StatusDot
            aria-label={agent.status}
            pulse={!terminal && agent.status === "running"}
            size="small"
            status={statusFor(agent)}
          />
          <button
            type="button"
            className={styles.nodeTitle}
            disabled={!agent.child_chat_id}
            title={rawTitle}
            onClick={() => {
              navigate();
            }}
          >
            {title}
          </button>
          <div className={styles.nodeActions}>
            {!terminal && (
              <Tooltip content="Send message">
                <IconButton
                  aria-expanded={messageOpen}
                  aria-label={`Message ${rawTitle}`}
                  icon={MessageSquare}
                  size="sm"
                  variant="plain"
                  onClick={(event) => {
                    event.stopPropagation();
                    setMessageOpen((value) => !value);
                  }}
                />
              </Tooltip>
            )}
            <Tooltip content={copied === "id" ? "Copied" : "Copy agent ID"}>
              <IconButton
                aria-label="Copy agent ID"
                icon={copied === "id" ? Check : Copy}
                size="sm"
                variant="plain"
                onClick={(event) => {
                  event.stopPropagation();
                  handleCopy("id", agent.agent_id);
                }}
              />
            </Tooltip>
            {agent.worktree_branch && (
              <Tooltip content={copied === "branch" ? "Copied" : "Copy branch"}>
                <IconButton
                  aria-label="Copy branch"
                  icon={copied === "branch" ? Check : Copy}
                  size="sm"
                  variant="plain"
                  onClick={(event) => {
                    event.stopPropagation();
                    handleCopy("branch", agent.worktree_branch ?? "");
                  }}
                />
              </Tooltip>
            )}
            {!terminal && (
              <Popover>
                <Popover.Trigger asChild>
                  <IconButton
                    aria-label={`Cancel ${rawTitle}`}
                    disabled={cancelState.isLoading}
                    icon={CircleStop}
                    onClick={(event) => event.stopPropagation()}
                    size="sm"
                    variant="danger"
                  />
                </Popover.Trigger>
                <Popover.Content maxWidth="280px">
                  <div className={styles.confirmPopover}>
                    <strong>Cancel this agent subtree?</strong>
                    <span>Running child agents will be cancelled too.</span>
                    <div className={styles.confirmActions}>
                      <Popover.Close asChild>
                        <Button size="sm" variant="soft">
                          Keep running
                        </Button>
                      </Popover.Close>
                      <Popover.Close asChild>
                        <Button
                          size="sm"
                          variant="danger"
                          onClick={() => void handleCancel()}
                        >
                          Cancel subtree
                        </Button>
                      </Popover.Close>
                    </div>
                  </div>
                </Popover.Content>
              </Popover>
            )}
          </div>
        </div>
        <div className={styles.metaLine}>
          <Badge
            className={styles.modelChip}
            tone="muted"
            title={agent.model_type ?? agent.kind}
          >
            {agent.model_type ?? agent.kind}
          </Badge>
          <span className={styles.nodeUsage}>
            {formatTokens(agent.tokens_used)}
            {cost ? ` · ${cost}` : ""}
          </span>
          {(agent.pending_questions ?? 0) > 0 && (
            <Badge
              className={styles.questionsBadge}
              tone="warning"
              title="Pending questions"
            >
              {agent.pending_questions}
            </Badge>
          )}
          {hasChildren && (
            <button
              type="button"
              aria-expanded={expanded}
              aria-label={`${expanded ? "Collapse" : "Expand"} ${rawTitle}`}
              className={styles.expandChip}
              onClick={(event) => {
                event.stopPropagation();
                setExpanded((value) => !value);
              }}
            >
              <ChevronRight
                aria-hidden="true"
                className={styles.expandChevron}
                size={12}
              />
              {`${children.length} agents`}
            </button>
          )}
        </div>
        {!terminal && agent.current_tool && (
          <div className={styles.toolTicker} title={agent.current_tool}>
            {agent.current_tool}
          </div>
        )}
      </div>
      {renderMessageForm && (
        <div
          className="rf-expand-grid"
          data-open={messageMotion.isAnimatingOpen}
        >
          <div>
            <form
              className={styles.messageForm}
              onSubmit={(event) => {
                event.preventDefault();
                void handleMessage();
              }}
            >
              <input
                aria-label={`Message for ${rawTitle}`}
                autoFocus
                className={styles.messageInput}
                placeholder="Send a message"
                value={message}
                onChange={(event) => setMessage(event.target.value)}
              />
              <IconButton
                aria-label="Send message"
                disabled={!message.trim()}
                icon={Send}
                loading={messageState.isLoading}
                size="sm"
                type="submit"
                variant="primary"
              />
            </form>
          </div>
        </div>
      )}
      {renderDetails && (
        <div
          className="rf-expand-grid"
          data-open={expandedMotion.isAnimatingOpen}
        >
          <div>
            <div className={styles.nodeDetails}>
              {agent.progress && <p>{agent.progress}</p>}
              {agent.goal_summary && <p>{agent.goal_summary}</p>}
              {files.length > 0 && (
                <div className={styles.fileList}>
                  {files.map((file) => (
                    <Badge key={file} tone="muted" title={file}>
                      {file}
                    </Badge>
                  ))}
                </div>
              )}
              {(agent.worktree_branch ?? agent.merge_status) && (
                <div className={styles.branchRow}>
                  {agent.worktree_branch && (
                    <span>{agent.worktree_branch}</span>
                  )}
                  {agent.merge_status && (
                    <Badge tone="muted">{agent.merge_status}</Badge>
                  )}
                </div>
              )}
              {(agent.started_at ??
                agent.finished_at ??
                agent.last_activity) && (
                <div className={styles.timestamps}>
                  {agent.started_at && (
                    <span>
                      Started {new Date(agent.started_at).toLocaleString()}
                    </span>
                  )}
                  {agent.last_activity && (
                    <span>
                      Active {new Date(agent.last_activity).toLocaleString()}
                    </span>
                  )}
                  {agent.finished_at && (
                    <span>
                      Finished {new Date(agent.finished_at).toLocaleString()}
                    </span>
                  )}
                </div>
              )}
              {questions.length > 0 && (
                <div className={styles.questions}>
                  {questions.map((question) => (
                    <div key={question.id} className={styles.question}>
                      <span>{question.text}</span>
                      <span>{question.answer ?? "Awaiting answer"}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
            {hasChildren && (
              <ul className={classNames(styles.children, "rf-stagger")}>
                {children.map((child) => (
                  <AgentTreeNode
                    chatId={chatId}
                    key={child.agent.agent_id}
                    depth={depth + 1}
                    node={child}
                    onNavigate={onNavigate}
                  />
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </li>
  );
}
