import React, { useCallback, useMemo } from "react";
import { Bot, ExternalLink, Telescope } from "lucide-react";
import { Button } from "../ui";
import {
  expandTruncatedAgentTitle,
  stripAgentKindPrefix,
} from "../../utils/displayNames";
import type {
  BackgroundAgentSummary,
  ToolCall,
} from "../../services/refact/types";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectBackgroundAgent,
  selectToolResultByThreadAndId,
} from "../../features/Chat/Thread/selectors";
import { parseAgenticArgs } from "../Tools/AgenticTool/agenticToolModel";
import { Markdown } from "../Markdown";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptRawBlock,
} from "../ChatContent/TranscriptItem";
import {
  FileChips,
  ReportMarkdown,
} from "../ChatContent/ToolCard/ReportToolCard";
import { extractSubagentReport } from "../ChatContent/ToolCard/subagentReport";
import { HandoffBlock, QuestionsBlock } from "./BackgroundAgentSections";
import {
  agentFacts,
  agentFiles,
  agentMeta,
  agentTranscriptStatus,
  isTerminal,
} from "./backgroundAgentPresentation";
import styles from "./BackgroundAgentCard.module.css";

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

function readTask(toolCall: ToolCall | undefined): string | undefined {
  if (!toolCall) return undefined;
  const task = parseAgenticArgs(toolCall.function.arguments).args.task;
  return typeof task === "string" ? task : undefined;
}

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
  onOpenTrajectory,
  onOpenAgentTrajectory,
}) => {
  const subscribedAgent = useAppSelector((state) =>
    selectBackgroundAgent(state, threadId ?? "", agentId ?? agent.agent_id),
  );
  const resolvedToolCallId = toolCallId ?? toolCall?.id;
  const toolResult = useAppSelector((state) =>
    resolvedToolCallId
      ? selectToolResultByThreadAndId(state, threadId ?? "", resolvedToolCallId)
      : undefined,
  );
  const merged = subscribedAgent ?? agent;
  const task = readTask(toolCall);
  const title = expandTruncatedAgentTitle(merged.title, task);
  const displayed = useMemo(
    () => (title === merged.title ? merged : { ...merged, title }),
    [merged, title],
  );
  const inline = presentation === "inline";
  const known = inline ? Boolean(subscribedAgent) || statusKnown : true;
  const { status, label } = agentTranscriptStatus(
    displayed,
    known,
    inline && launchFailed,
  );

  const report = useMemo(() => {
    const content =
      typeof toolResult?.content === "string" ? toolResult.content : null;
    const legacy = content ? extractSubagentReport(content) : null;
    return legacy?.markdown ?? displayed.result_summary ?? null;
  }, [displayed.result_summary, toolResult?.content]);

  const handleOpenTrajectory = useCallback(() => {
    const childChatId = displayed.child_chat_id;
    if (!childChatId) return;
    if (onOpenAgentTrajectory) onOpenAgentTrajectory(displayed, childChatId);
    else onOpenTrajectory?.(childChatId);
  }, [displayed, onOpenAgentTrajectory, onOpenTrajectory]);
  const canOpen =
    Boolean(displayed.child_chat_id) &&
    Boolean(onOpenTrajectory ?? onOpenAgentTrajectory);

  const files = agentFiles(displayed);
  const questions = displayed.questions ?? [];
  const terminal = isTerminal(displayed);
  const inputTask = task ?? displayed.goal_summary ?? null;
  const progress = terminal ? null : displayed.progress;

  const output = report ? (
    <ReportMarkdown markdown={report} />
  ) : progress ? (
    <p className={styles.muted}>{progress}</p>
  ) : null;

  const artifacts =
    files.length > 0 || terminal || questions.length > 0 ? (
      <div className={styles.artifacts}>
        <FileChips files={files} />
        {terminal && <HandoffBlock agent={displayed} />}
        <QuestionsBlock questions={questions} />
      </div>
    ) : null;

  const raw: TranscriptRawBlock[] = [
    ...(toolCall
      ? [{ label: "Launch arguments", value: toolCall.function.arguments }]
      : []),
    ...(toolResult
      ? [{ label: "Launch response", value: toolResult.content }]
      : []),
    ...(toolResult?.extra
      ? [{ label: "Response metadata", value: toolResult.extra }]
      : []),
    ...(displayed.handoff
      ? [{ label: "Handoff", value: displayed.handoff }]
      : []),
    { label: "Agent", value: displayed.agent_id },
  ];

  const Icon = displayed.kind === "delegate" ? Bot : Telescope;

  return (
    <TranscriptItem
      data-agent-kind={displayed.kind}
      data-testid={inline ? "background-agent-inline" : "background-agent-card"}
      actions={
        canOpen ? (
          <Button size="sm" variant="soft" onClick={handleOpenTrajectory}>
            <ExternalLink />
            Open trajectory
          </Button>
        ) : null
      }
      artifacts={artifacts}
      defaultOpen={status === "needs_you" || (!inline && !compactDefault)}
      error={displayed.error}
      icon={<Icon />}
      input={
        inputTask ? (
          <TranscriptClamp>
            <Markdown>{inputTask}</Markdown>
          </TranscriptClamp>
        ) : null
      }
      meta={agentMeta(displayed)}
      openKey={
        inline && resolvedToolCallId
          ? `tc:${resolvedToolCallId}`
          : `background-agent:${displayed.agent_id}`
      }
      output={output}
      raw={raw}
      rowTooltip={displayed.title}
      sectionLabels={{
        input: "Task",
        output: report ? "Report" : "Progress",
      }}
      status={status}
      statusLabel={label}
      summary={agentFacts(displayed)}
      target={stripAgentKindPrefix(displayed.title) || "Background work"}
      verb={displayed.kind === "delegate" ? "Delegate" : "Subagent"}
    />
  );
};

export const BackgroundAgentCard = React.memo(_BackgroundAgentCard);
