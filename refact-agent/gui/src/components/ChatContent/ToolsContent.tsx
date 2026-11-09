import React, { useCallback, useMemo } from "react";
import {
  ChatContextFile,
  DiffChunk,
  BackgroundAgentSummary,
  isMultiModalToolResult,
  extractExecMetadata,
  ToolCall,
  ToolResult,
} from "../../services/refact";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectManyToolResultsByThreadAndIds } from "../../features/Chat/Thread/selectors";
import { ChatThreadProvider, useThreadId } from "../../features/Chat/Thread";
import { RootState } from "../../app/store";
import { createChatWithId, switchToThread } from "../../features/Chat/Thread";
import { selectFeatures } from "../../features/Config/configSlice";
import { push } from "../../features/Pages/pagesSlice";
import { isRawTextDocToolCall } from "../Tools/types";
import { normalizeToolCall } from "../../utils/toolNameAliases";
import {
  ReadTool,
  ListTool,
  SearchTool,
  WebTool,
  KnowledgeTool,
  ShellTool as NewShellTool,
  ExecToolCard,
  SubagentTool as NewSubagentTool,
  EngineAnalysisTool,
  ShellServiceTool as NewShellServiceTool,
  EditTool,
  FileOpTool,
  TasksTool,
  GenericTool,
  TaskDoneTool,
  AskQuestionsTool,
  ChromeTool,
  SleepToolCard,
  OpenAIResponsesTool,
  OpenAIWebSearchCallTool,
  OpenAIFileSearchCallTool,
  OpenAICodeInterpreterCallTool,
  OpenAIComputerCallTool,
  OpenAIComputerCallOutputTool,
  OpenAIImageGenerationCallTool,
  OpenAIAudioTool,
  OpenAIRefusalTool,
  OpenAIMcpCallTool,
  OpenAIMcpListToolsTool,
  CompressReportTool,
} from "./ToolCard";
import { MultiModalTool } from "./ToolCard/MultiModalTool";
import { AppTool } from "./ToolCard/AppTool";
import { isAppToolName } from "./ToolCard/appToolModel";
import { AgentStatusView } from "./AgentStatusView";
import { AgentPulseView } from "./AgentPulseView";
import { AgentDiffView } from "./AgentDiffView";
import { TaskDocumentsView } from "./TaskDocumentsView";
import { FinalReportTool } from "./FinalReportView";
import { BackgroundAgentCard } from "../BackgroundAgentCard";
import { AgenticTool, isAgenticToolName } from "../Tools/AgenticTool";
import { LaunchPreviewTool } from "../Tools/AgenticTool/LaunchPreviewTool";
import { parseAgenticArgs } from "../Tools/AgenticTool/agenticToolModel";
import { expandTruncatedAgentTitle } from "../../utils/displayNames";
import { PrivacyToolResult } from "../../features/Privacy/PrivacyToolResult";
import {
  extractPrivacyShellMetadata,
  isPrivacyRefusalContent,
} from "../../services/refact/privacy";

const EXEC_TOOL_NAMES = new Set([
  "process_start",
  "process_list",
  "process_read",
  "process_kill",
  "process_wait",
  "process_write_stdin",
] as const);

const ENGINE_ANALYSIS_TOOLS: ReadonlySet<string> = new Set([
  "code_duplication",
  "code_health",
  "code_map",
  "code_why",
  "codegraph_overview",
  "contrast_audit",
  "dead_code",
  "design_system",
  "git_risk",
  "image_region",
  "mark_elements",
  "pr_blast",
  "security_scan",
  "ui_probe",
  "visual_diff",
] as const);

type ProcessToolName =
  | "process_start"
  | "process_list"
  | "process_read"
  | "process_kill"
  | "process_wait"
  | "process_write_stdin";

function isProcessToolName(name: string | undefined): name is ProcessToolName {
  return (
    typeof name === "string" && EXEC_TOOL_NAMES.has(name as ProcessToolName)
  );
}

function hasExecMetadata(result: ToolResult | undefined): boolean {
  return extractExecMetadata(result?.extra) !== undefined;
}

type BackgroundAgentExtra = Partial<
  Pick<
    BackgroundAgentSummary,
    | "child_chat_id"
    | "kind"
    | "status"
    | "title"
    | "progress"
    | "step_count"
    | "last_activity"
    | "target_files"
    | "edited_files"
    | "diff_summary"
    | "conflict_summary"
    | "result_summary"
    | "error"
    | "started_at"
    | "finished_at"
    | "change_seq"
    | "model"
    | "model_type"
    | "current_tool"
    | "goal_summary"
    | "plan_present"
    | "worktree_branch"
    | "merge_status"
    | "pending_questions"
    | "questions"
    | "tokens_used"
    | "cost_usd"
  >
> & {
  background_agent_id?: string;
  background_agent_kind?: BackgroundAgentSummary["kind"];
  background_agent_status?: BackgroundAgentSummary["status"];
  parent_chat_id?: string;
};

function isBackgroundAgentTool(
  toolName: string | undefined,
): toolName is "subagent" | "delegate" {
  return toolName === "subagent" || toolName === "delegate";
}

function getBackgroundAgentId(result: ToolResult | undefined) {
  return readStringField(result, "background_agent_id");
}

function readTopLevelBackgroundAgentValue(
  result: ToolResult | undefined,
  key: keyof BackgroundAgentExtra,
): unknown {
  const top = result as unknown as Record<string, unknown> | undefined;
  return top?.[key];
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function readStringField(
  result: ToolResult | undefined,
  key: keyof BackgroundAgentExtra,
): string | null {
  const direct = readTopLevelBackgroundAgentValue(result, key);
  if (typeof direct === "string") return direct;
  const fromExtra = result?.extra?.[key];
  return typeof fromExtra === "string" ? fromExtra : null;
}

function readNumberField(
  result: ToolResult | undefined,
  key: keyof BackgroundAgentExtra,
): number | null {
  const direct = readTopLevelBackgroundAgentValue(result, key);
  if (typeof direct === "number") return direct;
  const fromExtra = result?.extra?.[key];
  return typeof fromExtra === "number" ? fromExtra : null;
}

function readBooleanField(
  result: ToolResult | undefined,
  key: keyof BackgroundAgentExtra,
): boolean | undefined {
  const direct = readTopLevelBackgroundAgentValue(result, key);
  if (typeof direct === "boolean") return direct;
  const fromExtra = result?.extra?.[key];
  return typeof fromExtra === "boolean" ? fromExtra : undefined;
}

function readMergeStatusField(
  result: ToolResult | undefined,
): BackgroundAgentSummary["merge_status"] {
  const value = readStringField(result, "merge_status");
  return value === "pending" ||
    value === "merged" ||
    value === "conflict" ||
    value === "skipped" ||
    value === "failed"
    ? value
    : null;
}

function readStringArrayField(
  result: ToolResult | undefined,
  key: keyof BackgroundAgentExtra,
): string[] {
  const direct = readTopLevelBackgroundAgentValue(result, key);
  if (isStringArray(direct)) return direct;
  const fromExtra = result?.extra?.[key];
  return isStringArray(fromExtra) ? fromExtra : [];
}

function readBackgroundAgentKind(
  result: ToolResult | undefined,
  toolName: "subagent" | "delegate",
): BackgroundAgentSummary["kind"] {
  const value =
    readStringField(result, "background_agent_kind") ??
    readStringField(result, "kind");
  if (value === "subagent" || value === "delegate") return value;
  return toolName === "delegate" ? "delegate" : "subagent";
}

function isBackgroundAgentStatus(
  value: string | null,
): value is BackgroundAgentSummary["status"] {
  return (
    value === "queued" ||
    value === "running" ||
    value === "waiting_for_approval" ||
    value === "completed" ||
    value === "failed" ||
    value === "cancelled" ||
    value === "interrupted"
  );
}

function readBackgroundAgentStatus(
  result: ToolResult | undefined,
): BackgroundAgentSummary["status"] {
  const value =
    readStringField(result, "background_agent_status") ??
    readStringField(result, "status");
  return isBackgroundAgentStatus(value) ? value : "queued";
}

function readBackgroundAgentTitle(
  result: ToolResult,
  task: unknown,
  agentId: string,
): string {
  const title = readStringField(result, "title");
  if (title) return expandTruncatedAgentTitle(title, task);
  return typeof task === "string" && task.trim() !== ""
    ? task
    : `Requested background work: ${agentId.slice(-8)}`;
}

function backgroundAgentPlaceholder(
  result: ToolResult,
  toolName: "subagent" | "delegate",
  toolCall: ToolCall,
): BackgroundAgentSummary | null {
  const agentId = getBackgroundAgentId(result);
  if (!agentId) return null;
  const kind = readBackgroundAgentKind(result, toolName);
  const task = parseAgenticArgs(toolCall.function.arguments).args.task;
  return {
    agent_id: agentId,
    parent_chat_id: readStringField(result, "parent_chat_id") ?? "",
    child_chat_id: readStringField(result, "child_chat_id"),
    kind,
    status: readBackgroundAgentStatus(result),
    title: readBackgroundAgentTitle(result, task, agentId),
    progress: readStringField(result, "progress"),
    step_count: readNumberField(result, "step_count") ?? 0,
    last_activity: readStringField(result, "last_activity"),
    target_files: readStringArrayField(result, "target_files"),
    edited_files: readStringArrayField(result, "edited_files"),
    diff_summary: readStringField(result, "diff_summary"),
    conflict_summary: readStringField(result, "conflict_summary"),
    result_summary: readStringField(result, "result_summary"),
    error: readStringField(result, "error"),
    started_at: readStringField(result, "started_at"),
    finished_at: readStringField(result, "finished_at"),
    change_seq: readNumberField(result, "change_seq") ?? 0,
    model: readStringField(result, "model"),
    model_type: readStringField(result, "model_type"),
    current_tool: readStringField(result, "current_tool"),
    goal_summary: readStringField(result, "goal_summary"),
    plan_present: readBooleanField(result, "plan_present") ?? false,
    worktree_branch: readStringField(result, "worktree_branch"),
    merge_status: readMergeStatusField(result),
    pending_questions:
      readNumberField(result, "pending_questions") ?? undefined,
    questions: [],
    tokens_used: readNumberField(result, "tokens_used") ?? 0,
    cost_usd: readNumberField(result, "cost_usd"),
  } satisfies BackgroundAgentSummary;
}

function decorateBackgroundAgentTool(
  elem: React.ReactNode,
  toolCall: ToolCall,
  toolName: string | undefined,
  result: ToolResult | undefined,
  threadId: string | undefined,
  onOpenTrajectory: (
    agent: BackgroundAgentSummary,
    childChatId: string,
  ) => void,
): React.ReactNode {
  if (!isBackgroundAgentTool(toolName)) return elem;
  const agentId = getBackgroundAgentId(result);
  if (!result || !agentId) return elem;
  const agent = backgroundAgentPlaceholder(result, toolName, toolCall);
  if (!agent) return elem;
  return (
    <BackgroundAgentCard
      key={`background-agent-${agent.agent_id}`}
      agent={agent}
      agentId={agentId}
      threadId={threadId}
      presentation="inline"
      toolCall={toolCall}
      launchFailed={result.tool_failed === true}
      statusKnown={isBackgroundAgentStatus(
        readStringField(result, "background_agent_status") ??
          readStringField(result, "status"),
      )}
      onOpenAgentTrajectory={onOpenTrajectory}
    />
  );
}

export type ToolContentProps = {
  toolCalls: ToolCall[];
  contextFilesByToolId?: Record<string, ChatContextFile[]>;
  diffsByToolId?: Record<string, DiffChunk[]>;
  isActiveAssistant?: boolean;
  threadId?: string;
};

const _ToolContent: React.FC<ToolContentProps> = ({
  toolCalls,
  contextFilesByToolId,
  diffsByToolId,
  isActiveAssistant = false,
  threadId,
}) => {
  const dispatch = useAppDispatch();
  const contextThreadId = useThreadId();
  const toolThreadId = threadId ?? contextThreadId;
  const features = useAppSelector(selectFeatures);
  const handleOpenTrajectory = useCallback(
    (agent: BackgroundAgentSummary, childChatId: string) => {
      dispatch(
        createChatWithId({
          id: childChatId,
          title: agent.title,
          parentId: agent.parent_chat_id || toolThreadId,
          linkType: agent.kind,
        }),
      );
      dispatch(switchToThread({ id: childChatId }));
      dispatch(push({ name: "chat" }));
    },
    [toolThreadId, dispatch],
  );
  const ids = useMemo(() => {
    const out: string[] = [];
    for (const toolCall of toolCalls) {
      if (toolCall.id !== undefined) {
        out.push(toolCall.id);
      }
    }
    return out;
  }, [toolCalls]);
  const idsKey = ids.join("|");
  const selectResults = useMemo(
    () => selectManyToolResultsByThreadAndIds(toolThreadId, ids),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [toolThreadId, idsKey],
  );
  const allToolResults = useAppSelector(selectResults);
  const activeToolCallId = isActiveAssistant ? ids[ids.length - 1] : undefined;

  return (
    <ChatThreadProvider chatId={toolThreadId}>
      {processToolCalls(
        toolCalls,
        allToolResults,
        features,
        [],
        contextFilesByToolId,
        diffsByToolId,
        activeToolCallId,
        handleOpenTrajectory,
        toolThreadId,
      )}
    </ChatThreadProvider>
  );
};

export const ToolContent = React.memo(_ToolContent);

function processToolCalls(
  toolCalls: ToolCall[],
  toolResults: ToolResult[],
  features: RootState["config"]["features"] = {},
  processed: React.ReactNode[] = [],
  contextFilesByToolId: Record<string, ChatContextFile[]> = {},
  diffsByToolId: Record<string, DiffChunk[]> = {},
  activeToolCallId?: string,
  onOpenTrajectory: (
    agent: BackgroundAgentSummary,
    childChatId: string,
  ) => void = () => undefined,
  threadId?: string,
) {
  if (toolCalls.length === 0) return processed;
  const [head, ...tail] = toolCalls;
  const normalizedHead = normalizeToolCall(head);
  const headName = normalizedHead.function.name;
  const result = toolResults.find((result) => result.tool_call_id === head.id);
  const contextFiles = head.id ? contextFilesByToolId[head.id] : undefined;
  const diffs = head.id ? diffsByToolId[head.id] : undefined;
  const isActiveTool = head.id === activeToolCallId;

  if (
    result &&
    head.id &&
    threadId &&
    (extractPrivacyShellMetadata(result.extra) !== null ||
      isPrivacyRefusalContent(result.content))
  ) {
    const elem = (
      <PrivacyToolResult
        key={`privacy-tool-result-${head.id}`}
        threadId={threadId}
        toolCallId={head.id}
        result={result}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  // Privacy must win before family routing, including multimodal App captures.
  if (isAppToolName(headName)) {
    const elem = (
      <AppTool
        key={`app-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        result={result}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "cat") {
    const elem = (
      <ReadTool
        key={`read-tool-${processed.length}`}
        toolCall={normalizedHead}
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "tree") {
    const elem = (
      <ListTool
        key={`list-tool-${processed.length}`}
        toolCall={normalizedHead}
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "search_pattern") {
    const elem = (
      <SearchTool
        key={`search-pattern-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="search_pattern"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "search_semantic") {
    const elem = (
      <SearchTool
        key={`search-semantic-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="search_semantic"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "search_symbol_definition") {
    const elem = (
      <SearchTool
        key={`search-symbol-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="search_symbol_definition"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "glob") {
    const elem = (
      <SearchTool
        key={`glob-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="glob"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (isProcessToolName(headName)) {
    const elem = (
      <ExecToolCard
        key={`exec-tool-${headName}-${processed.length}`}
        toolCall={normalizedHead}
        toolName={headName}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "shell") {
    const elem = (
      <NewShellTool
        key={`shell-tool-${processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "shell_service") {
    const elem = (
      <NewShellServiceTool
        key={`shell-service-tool-${processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "subagent" || headName === "delegate") {
    const isPreview =
      headName === "subagent" &&
      parseAgenticArgs(normalizedHead.function.arguments).args.action ===
        "preview";
    const elem = isPreview ? (
      <LaunchPreviewTool
        key={`launch-preview-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        rawContent={result?.content}
        extra={result?.extra}
        hasResult={Boolean(result)}
        failed={result?.tool_failed === true}
      />
    ) : (
      <NewSubagentTool
        key={`background-agent-tool-${headName}-${processed.length}`}
        toolCall={normalizedHead}
        presentation="inline"
      />
    );
    const decoratedElem = isPreview
      ? elem
      : decorateBackgroundAgentTool(
          elem,
          normalizedHead,
          headName,
          result,
          threadId,
          onOpenTrajectory,
        );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, decoratedElem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "knowledge") {
    const elem = (
      <KnowledgeTool
        key={`knowledge-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="knowledge"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "search_trajectories") {
    const elem = (
      <KnowledgeTool
        key={`trajectories-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="search_trajectories"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "get_trajectory_context") {
    const elem = (
      <KnowledgeTool
        key={`trajectory-context-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="trajectories"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "create_knowledge") {
    const elem = (
      <KnowledgeTool
        key={`create-knowledge-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="create_knowledge"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "activate_skill") {
    return processToolCalls(
      tail,
      toolResults,
      features,
      processed,
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "web") {
    const elem = (
      <WebTool
        key={`web-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="web"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "web_search") {
    const elem = (
      <WebTool
        key={`web-search-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="web_search"
        contextFiles={contextFiles}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (isRawTextDocToolCall(normalizedHead)) {
    const elem = (
      <EditTool
        key={`edit-tool-${headName}-${processed.length}`}
        toolCall={normalizedHead}
        diffs={diffs}
        isActiveTool={isActiveTool}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "mv") {
    const elem = (
      <FileOpTool
        key={`mv-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="mv"
        isActiveTool={isActiveTool}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "rm") {
    const elem = (
      <FileOpTool
        key={`rm-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="rm"
        diffs={diffs}
        isActiveTool={isActiveTool}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "add_workspace_folder") {
    const elem = (
      <FileOpTool
        key={`add-workspace-tool-${processed.length}`}
        toolCall={normalizedHead}
        toolType="add_workspace_folder"
        isActiveTool={isActiveTool}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "tasks_set") {
    const elem = (
      <TasksTool
        key={`tasks-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "check_agents") {
    const elem = (
      <AgentStatusView
        key={`agent-status-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "agent_pulse") {
    const elem = (
      <AgentPulseView
        key={`agent-pulse-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "agent_diff") {
    const elem = (
      <AgentDiffView
        key={`agent-diff-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "doc_list" || headName === "doc_get") {
    const elem = (
      <TaskDocumentsView
        key={`task-documents-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        toolType={headName}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "agent_finish") {
    const elem = (
      <FinalReportTool
        key={`final-report-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        content={typeof result?.content === "string" ? result.content : null}
        hasResult={Boolean(result)}
        toolFailed={result?.tool_failed}
        rawContent={result?.content}
        extra={result?.extra}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "task_done") {
    const elem = (
      <TaskDoneTool
        key={`task-done-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "ask_questions") {
    const elem = (
      <AskQuestionsTool
        key={`ask-questions-tool-${processed.length}`}
        toolCall={normalizedHead}
        compact
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (isAgenticToolName(headName)) {
    const elem = (
      <AgenticTool
        key={`agentic-tool-${headName}-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        threadId={threadId}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName?.startsWith("openai_")) {
    const name = headName;
    let elem: React.ReactNode;
    switch (name) {
      case "openai_web_search_call":
        elem = (
          <OpenAIWebSearchCallTool
            key={`openai-web-search-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_file_search_call":
        elem = (
          <OpenAIFileSearchCallTool
            key={`openai-file-search-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_code_interpreter_call":
        elem = (
          <OpenAICodeInterpreterCallTool
            key={`openai-code-interpreter-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_computer_call":
        elem = (
          <OpenAIComputerCallTool
            key={`openai-computer-call-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_computer_call_output":
        elem = (
          <OpenAIComputerCallOutputTool
            key={`openai-computer-output-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_image_generation_call":
        elem = (
          <OpenAIImageGenerationCallTool
            key={`openai-image-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_audio":
        elem = (
          <OpenAIAudioTool
            key={`openai-audio-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_refusal":
        elem = (
          <OpenAIRefusalTool
            key={`openai-refusal-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_mcp_call":
        elem = (
          <OpenAIMcpCallTool
            key={`openai-mcp-call-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      case "openai_mcp_list_tools":
        elem = (
          <OpenAIMcpListToolsTool
            key={`openai-mcp-list-tools-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
        break;
      default:
        elem = (
          <OpenAIResponsesTool
            key={`openai-responses-tool-${head.id ?? processed.length}`}
            toolCall={normalizedHead}
          />
        );
    }

    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (
    headName === "compress_chat_probe" ||
    headName === "compress_chat_apply"
  ) {
    const elem = (
      <CompressReportTool
        key={`compress-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        toolType={
          headName === "compress_chat_probe" ? "ctx_probe" : "ctx_apply"
        }
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "sleep") {
    const elem = (
      <SleepToolCard
        key={`sleep-tool-${processed.length}`}
        toolCall={normalizedHead}
        threadId={threadId}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "chrome") {
    const elem = (
      <ChromeTool
        key={`chrome-tool-${processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (hasExecMetadata(result)) {
    const elem = (
      <ExecToolCard
        key={`exec-metadata-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        toolName="exec"
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (result && isMultiModalToolResult(result)) {
    const elem = (
      <MultiModalTool
        key={`multimodal-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
        result={result}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName && ENGINE_ANALYSIS_TOOLS.has(headName)) {
    const elem = (
      <EngineAnalysisTool
        key={`generic-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      onOpenTrajectory,
      threadId,
    );
  }

  const elem = (
    <GenericTool
      key={`generic-tool-${head.id ?? processed.length}`}
      toolCall={normalizedHead}
    />
  );
  return processToolCalls(
    tail,
    toolResults,
    features,
    [...processed, elem],
    contextFilesByToolId,
    diffsByToolId,
    activeToolCallId,
    onOpenTrajectory,
    threadId,
  );
}
