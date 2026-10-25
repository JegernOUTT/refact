import React, { useCallback, useMemo } from "react";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { useAppSelector } from "../../../hooks/useAppSelector";
import {
  selectIsStreamingById,
  selectIsWaitingById,
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
import { Badge, Icon, ToolCard as CollaborationCard } from "../../ui";
import { ToolCard, type ToolStatus } from "../../ChatContent/ToolCard";
import { ToolCallTooltip } from "../../ChatContent/ToolCard/ToolCallTooltip";
import { useStoredOpen } from "../../ChatContent/useStoredOpen";
import {
  useChatScrollAnchor,
  usePrepareChatScrollAnchor,
} from "../../ChatContent/useChatScrollAnchor";
import {
  buildAgenticToolViewModel,
  type AgenticToolViewModel,
} from "./agenticToolModel";
import { agenticIcon } from "./agenticIcons";
import { WorkToolView } from "./WorkToolView";
import { ApprovalToolView } from "./ApprovalToolView";
import { ResumeToolView } from "./ResumeToolView";
import { PlanToolView } from "./PlanToolView";
import { GoalToolView } from "./GoalToolView";
import { MessageToolView } from "./MessageToolView";
import { SpawnToolView } from "./SpawnToolView";
import { OverviewToolView } from "./OverviewToolView";
import { InspectToolView } from "./InspectToolView";
import { ControlToolView } from "./ControlToolView";
import { BoardToolView } from "./BoardToolView";
import { DocToolView } from "./DocToolView";
import { MemoryToolView } from "./MemoryToolView";
import { buildCompactCollaborationPresentation } from "./compactCollaboration";
import { CompactToolDetails } from "./CompactToolDetails";
import { PagedText } from "./PagedText";
import {
  LOCAL_COLLABORATION_TOOL_NAMES,
  lookupAgenticFamily,
} from "./agenticFamilies";

export type AgenticToolProps = {
  toolCall: ToolCall;
  threadId?: string;
};

function familyBody({
  model,
  content,
  extra,
  argumentsRaw,
  onOpenChildTrajectory,
}: {
  model: AgenticToolViewModel;
  content: string | null;
  extra: Record<string, unknown> | undefined;
  argumentsRaw: string | undefined;
  onOpenChildTrajectory: (childChatId: string) => void;
}): React.ReactNode {
  const name = normalizeToolName(model.toolName) ?? model.toolName;
  const pending = model.status === "pending";
  const failed = model.status === "failed";
  if (name === "work_overview" || name === "work_inspect")
    return <WorkToolView content={content} pending={pending} failed={failed} />;
  if (name === "agent_approval")
    return (
      <ApprovalToolView content={content} pending={pending} failed={failed} />
    );
  if (name === "agent_resume")
    return (
      <ResumeToolView
        content={content}
        pending={pending}
        failed={failed}
        onOpenChildTrajectory={onOpenChildTrajectory}
      />
    );
  if (name === "pause_goal" || name === "snooze_goal")
    return (
      <GoalToolView
        toolName={name}
        content={content}
        extra={extra}
        pending={pending}
        failed={failed}
      />
    );
  switch (model.family) {
    case "goal":
      return (
        <GoalToolView
          toolName={name}
          content={content}
          extra={extra}
          pending={pending}
          failed={failed}
        />
      );
    case "plan":
      return (
        <PlanToolView
          toolName={name}
          content={content}
          pending={pending}
          failed={failed}
        />
      );
    case "message":
    case "progress":
      return (
        <MessageToolView
          toolName={name}
          argumentsRaw={argumentsRaw}
          content={content}
          extra={extra}
          pending={pending}
          failed={failed}
        />
      );
    case "spawn":
      return (
        <SpawnToolView
          model={model}
          content={content}
          onOpenChildTrajectory={onOpenChildTrajectory}
        />
      );
    case "overview":
      return <OverviewToolView model={model} content={content} />;
    case "inspect":
      return (
        <InspectToolView
          model={model}
          content={content}
          onOpenChildTrajectory={onOpenChildTrajectory}
        />
      );
    case "control":
      return <ControlToolView model={model} content={content} />;
    case "task":
      return <BoardToolView model={model} content={content} />;
    case "doc":
      return <DocToolView model={model} content={content} />;
    case "memory":
      return <MemoryToolView model={model} content={content} />;
  }
}

export const AgenticTool: React.FC<AgenticToolProps> = ({
  toolCall,
  threadId,
}) => {
  const contextThreadId = useThreadId();
  const resolvedThreadId = threadId ?? contextThreadId;
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const isCollaboration = LOCAL_COLLABORATION_TOOL_NAMES.includes(
    normalizeToolName(toolCall.function.name) ?? "",
  );
  const [isOpen, handleToggle] = useStoredOpen(storeKey, !isCollaboration);
  const dispatch = useAppDispatch();

  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, resolvedThreadId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, resolvedThreadId),
  );
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, resolvedThreadId, toolCall.id),
  );

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const collaboration = useMemo(
    () =>
      buildCompactCollaborationPresentation({
        toolName: toolCall.function.name ?? "",
        arguments: toolCall.function.arguments,
        result: maybeResult?.content,
        extra: maybeResult?.extra,
        transportState: maybeResult
          ? maybeResult.tool_failed
            ? "failed"
            : "succeeded"
          : isStreaming || isWaiting
            ? "running"
            : "pending",
      }),
    [
      isStreaming,
      isWaiting,
      maybeResult,
      toolCall.function.arguments,
      toolCall.function.name,
    ],
  );

  const model = useMemo(
    () =>
      collaboration
        ? null
        : buildAgenticToolViewModel({
            toolName: toolCall.function.name,
            argumentsRaw: toolCall.function.arguments,
            content,
            toolFailed: maybeResult?.tool_failed,
            hasResult: Boolean(maybeResult),
            isBusy: isStreaming || isWaiting,
            historical: Boolean(maybeResult) && !isStreaming && !isWaiting,
          }),
    [
      collaboration,
      content,
      isStreaming,
      isWaiting,
      maybeResult,
      toolCall.function.arguments,
      toolCall.function.name,
    ],
  );

  const handleOpenChild = useCallback(
    (childChatId: string) => {
      dispatch(
        createChatWithId({
          id: childChatId,
          title: model?.headline ?? "Agent trajectory",
          parentId: resolvedThreadId,
          linkType: "subagent",
        }),
      );
      dispatch(switchToThread({ id: childChatId }));
      dispatch(push({ name: "chat" }));
    },
    [dispatch, model?.headline, resolvedThreadId],
  );

  const preserveScrollAnchor = useChatScrollAnchor();
  const prepareScrollAnchor = usePrepareChatScrollAnchor();
  const handleCollaborationToggle = useCallback(() => {
    preserveScrollAnchor(handleToggle);
  }, [handleToggle, preserveScrollAnchor]);
  const family = lookupAgenticFamily(toolCall.function.name);

  if (collaboration && family)
    return (
      <>
        <span data-testid="agentic-tool" hidden />
        <ToolCallTooltip toolCall={toolCall}>
          <CollaborationCard
            aria-label="Collaboration receipt"
            density="compact"
            icon={agenticIcon(family.icon)}
            title={collaboration.headline}
            status={
              collaboration.tone === "danger"
                ? "error"
                : collaboration.tone === "active"
                  ? "running"
                  : "idle"
            }
            open={isOpen}
            onPointerDownCapture={prepareScrollAnchor}
            onMouseDownCapture={prepareScrollAnchor}
            onKeyDownCapture={prepareScrollAnchor}
            onOpenChange={handleCollaborationToggle}
            actions={
              <Badge
                role={collaboration.tone === "danger" ? "alert" : "status"}
                tone={
                  collaboration.tone === "danger" ||
                  collaboration.tone === "warning"
                    ? collaboration.tone
                    : "muted"
                }
              >
                {collaboration.stateLabel}
              </Badge>
            }
          >
            <CompactToolDetails facts={collaboration.facts}>
              <PagedText
                label="Raw arguments"
                body={toolCall.function.arguments}
              />
              {maybeResult && (
                <PagedText
                  label="Full response"
                  body={content ?? JSON.stringify(maybeResult.content, null, 2)}
                />
              )}
              {maybeResult?.extra && (
                <PagedText
                  label="Response metadata"
                  body={JSON.stringify(maybeResult.extra, null, 2)}
                />
              )}
            </CompactToolDetails>
          </CollaborationCard>
        </ToolCallTooltip>
      </>
    );

  if (!model) return null;

  const status: ToolStatus =
    model.status === "pending"
      ? "running"
      : model.status === "failed"
        ? "error"
        : "success";

  return (
    <>
      <span data-testid="agentic-tool" hidden />
      <ToolCard
        icon={<Icon icon={agenticIcon(model.icon)} size="sm" />}
        summary={model.headline}
        meta={model.statusLabel ?? undefined}
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        {familyBody({
          model,
          content,
          extra: maybeResult?.extra,
          argumentsRaw: toolCall.function.arguments,
          onOpenChildTrajectory: handleOpenChild,
        })}
      </ToolCard>
    </>
  );
};

export default AgenticTool;
