import React, { useCallback, useMemo } from "react";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { useAppSelector } from "../../../hooks/useAppSelector";
import {
  selectBackgroundAgentPool,
  selectToolResultByThreadAndId,
} from "../../../features/Chat/Thread/selectors";
import {
  createChatWithId,
  switchToThread,
  useThreadId,
} from "../../../features/Chat/Thread";
import { push } from "../../../features/Pages/pagesSlice";
import type { ToolCall } from "../../../services/refact/types";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { Button } from "../../ui";
import { Markdown } from "../../Markdown";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptRawBlock,
} from "../../ChatContent/TranscriptItem";
import { useUnrecordedResultState } from "../../ChatContent/TranscriptItem/useUnrecordedResultState";
import {
  buildAgenticToolViewModel,
  parseAgenticArgs,
} from "./agenticToolModel";
import { agenticIcon } from "./agenticIcons";
import { lookupAgenticFamily } from "./agenticFamilies";
import { buildAgenticRow } from "./agenticRow";
import type { AgentLookup, OverviewAgentRow } from "./agenticRowShared";
import { AgenticFamilyView } from "./AgenticFamilyView";
import type { CompactTransportState } from "./compactPresentation";
import styles from "./AgenticTool.module.css";

export type AgenticToolProps = {
  toolCall: ToolCall;
  threadId?: string;
};

function AgentList({
  agents,
  onOpen,
}: {
  agents: OverviewAgentRow[];
  onOpen: (chatId: string, title: string) => void;
}) {
  return (
    <ul className={styles.agentList} data-testid="agentic-tool-agents">
      {agents.map((agent) => (
        <li
          className={styles.agentRow}
          key={agent.agentId}
          title={agent.agentId}
        >
          <span className={styles.agentState} data-status={agent.status}>
            {agent.state}
          </span>
          <span className={styles.agentTitle}>{agent.title}</span>
          {agent.step !== null && (
            <span className={styles.agentDetail}>step {agent.step}</span>
          )}
          {agent.childChatId !== null && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                if (agent.childChatId) onOpen(agent.childChatId, agent.title);
              }}
            >
              Open
            </Button>
          )}
        </li>
      ))}
    </ul>
  );
}

export const AgenticTool: React.FC<AgenticToolProps> = ({
  toolCall,
  threadId,
}) => {
  const contextThreadId = useThreadId();
  const resolvedThreadId = threadId ?? contextThreadId;
  const dispatch = useAppDispatch();
  const unrecorded = useUnrecordedResultState(toolCall.id, resolvedThreadId);
  const isBusy = unrecorded.status === "running";
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, resolvedThreadId, toolCall.id),
  );
  const agentPool = useAppSelector(selectBackgroundAgentPool);

  const toolName = toolCall.function.name ?? "";
  const argumentsRaw = toolCall.function.arguments;
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const transport: CompactTransportState = maybeResult
    ? maybeResult.tool_failed
      ? "failed"
      : "succeeded"
    : isBusy
      ? "running"
      : "pending";

  const lookup = useCallback<AgentLookup>(
    (agentId) => {
      const agent = Object.values(agentPool).find(
        (candidate) =>
          candidate.agent_id.startsWith(agentId) ||
          candidate.child_chat_id === agentId,
      );
      return agent
        ? { title: agent.title, childChatId: agent.child_chat_id }
        : null;
    },
    [agentPool],
  );

  const row = useMemo(
    () =>
      buildAgenticRow({
        toolName,
        args: parseAgenticArgs(argumentsRaw).args,
        content,
        extra: maybeResult?.extra,
        transport,
        lookup,
        unrecorded,
      }),
    [
      argumentsRaw,
      content,
      lookup,
      maybeResult?.extra,
      toolName,
      transport,
      unrecorded,
    ],
  );

  const model = useMemo(
    () =>
      row.useFamilyView
        ? buildAgenticToolViewModel({
            toolName,
            argumentsRaw,
            content,
            toolFailed: maybeResult?.tool_failed,
            hasResult: Boolean(maybeResult),
            isBusy,
          })
        : null,
    [argumentsRaw, content, isBusy, maybeResult, row.useFamilyView, toolName],
  );

  const openChild = useCallback(
    (childChatId: string, title: string) => {
      dispatch(
        createChatWithId({
          id: childChatId,
          title,
          parentId: resolvedThreadId,
          linkType: "subagent",
        }),
      );
      dispatch(switchToThread({ id: childChatId }));
      dispatch(push({ name: "chat" }));
    },
    [dispatch, resolvedThreadId],
  );

  const family = lookupAgenticFamily(toolName);
  if (!family) return null;
  const name = normalizeToolName(toolName) ?? toolName;
  const childChatId = row.childChatId ?? model?.childChatId ?? null;
  const title = row.target ?? row.verb;

  const output = model ? (
    <AgenticFamilyView
      model={model}
      toolName={name}
      content={content}
      extra={maybeResult?.extra}
      argumentsRaw={argumentsRaw}
    />
  ) : row.markdown !== null || row.agents !== null || row.criteria !== null ? (
    <TranscriptClamp>
      {row.agents && <AgentList agents={row.agents} onOpen={openChild} />}
      {row.markdown !== null && (
        <div data-testid="agentic-tool-message">
          <Markdown canHaveInteractiveElements={false}>{row.markdown}</Markdown>
        </div>
      )}
      {row.criteria && (
        <ol aria-label="Criteria" className={styles.criteria}>
          {row.criteria.map((criterion, index) => (
            <li key={index}>{criterion}</li>
          ))}
        </ol>
      )}
    </TranscriptClamp>
  ) : null;

  const raw: TranscriptRawBlock[] = [
    { label: "Arguments", value: argumentsRaw },
  ];
  if (maybeResult)
    raw.push({ label: "Response", value: content ?? maybeResult.content });
  if (maybeResult?.extra)
    raw.push({ label: "Metadata", value: maybeResult.extra });

  const Icon = agenticIcon(family.icon);
  return (
    <TranscriptItem
      data-testid="agentic-tool"
      data-tool={name}
      icon={<Icon aria-hidden />}
      verb={row.verb}
      target={row.target}
      separator={row.separator}
      meta={row.meta}
      status={row.status}
      statusLabel={row.statusLabel}
      summary={row.facts}
      output={output}
      error={row.error}
      actions={
        childChatId ? (
          <Button
            size="sm"
            variant="soft"
            data-testid="agentic-tool-open-child"
            onClick={() => openChild(childChatId, title)}
          >
            Open child trajectory
          </Button>
        ) : null
      }
      raw={raw}
      openKey={toolCall.id ? `agentic:${toolCall.id}` : undefined}
      defaultOpen={row.status === "needs_you"}
    />
  );
};

export default AgenticTool;
