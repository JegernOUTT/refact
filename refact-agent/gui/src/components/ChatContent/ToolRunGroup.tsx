import React from "react";
import classNames from "classnames";
import { ChevronDown, Layers } from "lucide-react";
import type { BackgroundAgentSummary } from "../../services/refact";
import { shallowEqual } from "react-redux";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useReducedMotion } from "../../hooks/useReducedMotion";
import {
  selectManyToolResultsByThreadAndIds,
  selectThreadConfirmationById,
} from "../../features/Chat/Thread/selectors";
import type { EventMessage, ToolResult } from "../../services/refact";
import { normalizeToolName } from "../../utils/toolNameAliases";
import { agentTranscriptStatus } from "../BackgroundAgentCard/backgroundAgentPresentation";
import { useStoredOpen } from "./useStoredOpen";
import {
  TRANSCRIPT_COLLAPSE_FALLBACK_MS,
  isReducedMotionForced,
  useTranscriptCollapse,
} from "./TranscriptItem/useTranscriptCollapse";
import { useTranscriptAnchoredToggle } from "./TranscriptItem/useTranscriptAnchoredToggle";
import { useCollapsedFocusGuard } from "./TranscriptItem/useCollapsedFocusGuard";
import { selectFoldedEventIndexById } from "./foldedEventSelectors";
import { processIdFromStartResult } from "./ToolCard/execToolModel";
import type { DisplayItem } from "./ChatContentDisplayItems";
import {
  formatToolRunDuration,
  pinnedToolRunItem,
  summarizeToolRun,
  toolRunCallState,
  toolRunSummaryText,
  type DisplayItemToolRun,
  type ToolRunCallState,
  type ToolRunSummaryPart,
} from "./toolRunGroups";
import {
  PROCESS_OUTPUT_REVEAL_EVENT,
  isProcessOutputRevealDetail,
  revealProcessOutput,
} from "./revealProcessOutput";
import styles from "./ToolRunGroup.module.css";

type CallFlags = ToolRunCallState & { attention: boolean };

type BackgroundAgents = Partial<Record<string, BackgroundAgentSummary>>;

const NO_AGENTS: BackgroundAgents = {};

function agentNeedsAttention(
  agents: BackgroundAgents,
  result: ToolResult | undefined,
): boolean {
  const agentId =
    result?.background_agent_id ??
    (typeof result?.extra?.background_agent_id === "string"
      ? result.extra.background_agent_id
      : undefined);
  if (!agentId) return false;
  const agent = agents[agentId];
  return agent
    ? agentTranscriptStatus(agent, true, false).status === "needs_you"
    : false;
}

function computeCallFlags(
  group: DisplayItemToolRun,
  threadId: string,
  results: readonly ToolResult[],
  pausedIds: ReadonlySet<string>,
  foldedEvents: ReadonlyMap<string, EventMessage>,
  agents: BackgroundAgents,
): ReadonlyMap<string, CallFlags> {
  const resultsById = new Map(
    results.map((result) => [result.tool_call_id, result]),
  );
  const flags = new Map<string, CallFlags>();
  for (const { call, owner } of group.owners) {
    if (!call.id) continue;
    const result = resultsById.get(call.id);
    const callState = toolRunCallState(
      call,
      owner.message,
      result,
      foldedEvents.get(call.id),
      threadId,
    );
    const attention =
      pausedIds.has(call.id) ||
      callState.busy ||
      (normalizeToolName(call.function.name) === "subagent" &&
        agentNeedsAttention(agents, result));
    flags.set(call.id, { ...callState, attention });
  }
  return flags;
}

function SummaryPart({ part }: { part: ToolRunSummaryPart }) {
  return (
    <>
      <span className={styles.verb}>{part.verb}</span> {part.rest}
      {part.stats ? (
        <>
          {" "}
          <span className={styles.added}>+{part.stats.added}</span>{" "}
          <span className={styles.removed}>−{part.stats.removed}</span>
        </>
      ) : null}
    </>
  );
}

export type ToolRunGroupProps = {
  group: DisplayItemToolRun;
  threadId: string;
  renderMember: (item: DisplayItem) => React.ReactNode;
};

const _ToolRunGroup: React.FC<ToolRunGroupProps> = ({
  group,
  threadId,
  renderMember,
}) => {
  const [isOpen, , setOpen] = useStoredOpen(group.key, false);
  const calls = React.useMemo(
    () => group.owners.map((owner) => owner.call),
    [group.owners],
  );
  const ids = React.useMemo(
    () => calls.flatMap((call) => (call.id ? [call.id] : [])),
    [calls],
  );
  const selectResults = React.useMemo(
    () => selectManyToolResultsByThreadAndIds(threadId, ids),
    [threadId, ids],
  );
  const results = useAppSelector(selectResults);
  const pauseReasons = useAppSelector(
    (state) => selectThreadConfirmationById(state, threadId).pause_reasons,
  );
  const agents = useAppSelector(
    (state) => state.chat.threads[threadId]?.background_agents ?? NO_AGENTS,
  );
  const foldedList = useAppSelector((state) => {
    const index = selectFoldedEventIndexById(
      state,
      threadId,
    ).processEventByToolCallId;
    return ids.map((id) => index.get(id));
  }, shallowEqual);
  const flags = React.useMemo(() => {
    const folded = new Map<string, EventMessage>();
    ids.forEach((id, index) => {
      const event = foldedList[index];
      if (event) folded.set(id, event);
    });
    const paused = new Set(pauseReasons.map((reason) => reason.tool_call_id));
    return computeCallFlags(group, threadId, results, paused, folded, agents);
  }, [group, threadId, results, ids, foldedList, pauseReasons, agents]);
  const summary = React.useMemo(
    () => summarizeToolRun(calls, flags, group.diffsByToolId, group.cards),
    [calls, flags, group.diffsByToolId, group.cards],
  );
  const pinned = React.useMemo(
    () =>
      group.owners
        .filter(
          ({ call }) =>
            call.id !== undefined && flags.get(call.id)?.attention === true,
        )
        .map(({ call, owner }) => pinnedToolRunItem(owner, call)),
    [flags, group.owners],
  );

  const collapse = useTranscriptCollapse(isOpen);
  const [panelNode, setPanelNode] = React.useState<HTMLDivElement | null>(null);
  const panelRef = collapse.panelRef;
  const setPanelRef = React.useCallback(
    (node: HTMLDivElement | null) => {
      panelRef.current = node;
      setPanelNode(node);
    },
    [panelRef],
  );
  const guardRef = React.useMemo(() => ({ current: panelNode }), [panelNode]);
  useCollapsedFocusGuard(guardRef, !isOpen);
  const anchoredToggle = useTranscriptAnchoredToggle(panelRef);
  const toggleRef = React.useRef<HTMLButtonElement>(null);
  const panelId = React.useId();
  const metaId = React.useId();
  const reducedMotion = useReducedMotion();

  const toggle = React.useCallback(
    (trigger: HTMLElement) => {
      anchoredToggle.run(trigger, () => setOpen(!isOpen));
    },
    [anchoredToggle, isOpen, setOpen],
  );

  const handleRowClick = (event: React.MouseEvent<HTMLDivElement>) => {
    const button = toggleRef.current;
    if (!button) return;
    if (event.target instanceof Node && button.contains(event.target)) return;
    toggle(button);
  };

  const [revealTarget, setRevealTarget] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!isOpen) setRevealTarget(null);
  }, [isOpen]);
  React.useEffect(() => {
    const onReveal = (event: Event) => {
      if (!(event instanceof CustomEvent)) return;
      const detail: unknown = event.detail;
      if (!isProcessOutputRevealDetail(detail) || detail.claimed) return;
      const owns = results.some(
        (result) =>
          processIdFromStartResult(result.extra, result.content) ===
          detail.processId,
      );
      if (!owns) return;
      detail.claimed = true;
      setOpen(true);
      setRevealTarget(detail.processId);
    };
    window.addEventListener(PROCESS_OUTPUT_REVEAL_EVENT, onReveal);
    return () =>
      window.removeEventListener(PROCESS_OUTPUT_REVEAL_EVENT, onReveal);
  }, [results, setOpen]);

  React.useEffect(() => {
    if (!revealTarget || !collapse.expanded) return;
    let done = false;
    const reveal = () => {
      if (done) return;
      done = true;
      revealProcessOutput(revealTarget);
      setRevealTarget(null);
    };
    const onEnd = (event: TransitionEvent) => {
      if (event.target === panelNode) reveal();
    };
    panelNode?.addEventListener("transitionend", onEnd);
    const timer = window.setTimeout(
      reveal,
      reducedMotion || isReducedMotionForced()
        ? 0
        : TRANSCRIPT_COLLAPSE_FALLBACK_MS,
    );
    return () => {
      done = true;
      panelNode?.removeEventListener("transitionend", onEnd);
      window.clearTimeout(timer);
    };
  }, [revealTarget, collapse.expanded, panelNode, reducedMotion]);

  const summaryText = toolRunSummaryText(summary);
  const duration =
    summary.durationMs !== null
      ? formatToolRunDuration(summary.durationMs)
      : null;

  return (
    <section
      className={styles.root}
      data-open={isOpen}
      data-testid="tool-run-group"
      data-transcript-item="true"
    >
      <div
        className={styles.row}
        data-expandable="true"
        data-testid="tool-run-row"
        data-transcript-row=""
        onClick={handleRowClick}
      >
        <button
          aria-controls={panelId}
          aria-describedby={summary.failed > 0 || duration ? metaId : undefined}
          aria-expanded={isOpen}
          aria-label={summaryText}
          className={styles.toggle}
          data-testid="tool-run-toggle"
          ref={toggleRef}
          title={summaryText}
          type="button"
          onClick={(event) => toggle(event.currentTarget)}
          onKeyDown={anchoredToggle.prepare}
          onPointerDown={anchoredToggle.prepare}
        >
          <span className={styles.icon}>
            <Layers aria-hidden focusable="false" strokeWidth={1.5} />
          </span>
          <span className={styles.summary} data-testid="tool-run-summary">
            {summary.parts.map((part, index) => (
              <React.Fragment key={part.category}>
                {index > 0 ? " · " : null}
                <SummaryPart part={part} />
              </React.Fragment>
            ))}
          </span>
          <span className={styles.spacer} />
        </button>
        <span id={metaId} className={styles.metaGroup}>
          {summary.failed > 0 ? (
            <span
              className={styles.pill}
              data-tone="failed"
              data-visible="true"
              data-testid="tool-run-failed"
            >
              {summary.failed} failed
            </span>
          ) : null}
          {duration ? (
            <span className={styles.duration} data-testid="tool-run-duration">
              {duration}
            </span>
          ) : null}
        </span>
        <ChevronDown aria-hidden className={styles.chevron} />
      </div>
      {pinned.length > 0 && !collapse.mounted ? (
        <div className={styles.pinned} data-testid="tool-run-pinned">
          {pinned.map((item) => (
            <div className={styles.member} key={item.key}>
              {renderMember(item)}
            </div>
          ))}
        </div>
      ) : null}
      {collapse.mounted ? (
        <div
          className={classNames("rf-expand-grid", styles.panelGrid)}
          data-expanded={collapse.expanded}
          data-tool-run-panel=""
          data-open={isOpen}
          id={panelId}
          ref={setPanelRef}
          onTransitionEnd={collapse.onTransitionEnd}
        >
          <div className={styles.panelShell}>
            <div className={styles.members} data-testid="tool-run-members">
              {group.members.map((member) => (
                <div className={styles.member} key={member.key}>
                  {renderMember(member)}
                </div>
              ))}
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
};

export const ToolRunGroup = React.memo(_ToolRunGroup);
