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
import { Icon } from "../../ui";
import { ToolCard, type ToolStatus } from "../../ChatContent/ToolCard";
import { useStoredOpen } from "../../ChatContent/useStoredOpen";
import { buildAgenticToolViewModel } from "./agenticToolModel";
import { AgenticToolView } from "./AgenticToolView";
import { agenticIcon } from "./agenticIcons";
import { WorkToolView } from "./WorkToolView";
import { ApprovalToolView } from "./ApprovalToolView";
import { ResumeToolView } from "./ResumeToolView";
import { PlanToolView } from "./PlanToolView";
import { GoalToolView } from "./GoalToolView";
import { MessageToolView } from "./MessageToolView";

export type AgenticToolProps = {
  toolCall: ToolCall;
  threadId?: string;
};

export const AgenticTool: React.FC<AgenticToolProps> = ({
  toolCall,
  threadId,
}) => {
  const contextThreadId = useThreadId();
  const resolvedThreadId = threadId ?? contextThreadId;
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
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

  const model = useMemo(
    () =>
      buildAgenticToolViewModel({
        toolName: toolCall.function.name,
        argumentsRaw: toolCall.function.arguments,
        content,
        toolFailed: maybeResult?.tool_failed,
        hasResult: Boolean(maybeResult),
        isBusy: isStreaming || isWaiting,
        historical: Boolean(maybeResult) && !isStreaming && !isWaiting,
      }),
    [
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
        {model.toolName === "work_overview" ||
        model.toolName === "work_inspect" ? (
          <WorkToolView
            content={content}
            pending={model.status === "pending"}
            failed={model.status === "failed"}
          />
        ) : model.toolName === "agent_approval" ? (
          <ApprovalToolView
            content={content}
            pending={model.status === "pending"}
            failed={model.status === "failed"}
          />
        ) : model.toolName === "agent_resume" ? (
          <ResumeToolView
            content={content}
            pending={model.status === "pending"}
            failed={model.status === "failed"}
            onOpenChildTrajectory={handleOpenChild}
          />
        ) : model.family === "goal" ||
          model.toolName === "pause_goal" ||
          model.toolName === "snooze_goal" ? (
          <GoalToolView
            toolName={model.toolName}
            content={content}
            extra={maybeResult?.extra}
            pending={model.status === "pending"}
            failed={model.status === "failed"}
          />
        ) : model.family === "plan" ? (
          <PlanToolView
            toolName={model.toolName}
            content={content}
            pending={model.status === "pending"}
            failed={model.status === "failed"}
          />
        ) : model.family === "message" || model.family === "progress" ? (
          <MessageToolView
            toolName={model.toolName}
            argumentsRaw={toolCall.function.arguments}
            content={content}
            extra={maybeResult?.extra}
            pending={model.status === "pending"}
            failed={model.status === "failed"}
          />
        ) : (
          <AgenticToolView
            model={model}
            onOpenChildTrajectory={
              model.childChatId ? handleOpenChild : undefined
            }
          />
        )}
      </ToolCard>
    </>
  );
};

export default AgenticTool;
