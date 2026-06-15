import {
  CircleCheck,
  CircleX,
  File,
  LoaderCircle,
  Settings,
  Rows3,
} from "lucide-react";
import React, { forwardRef, useCallback, useEffect, useMemo } from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import classNames from "classnames";
import { Container, Flex, Text, Box } from "@radix-ui/themes";
import { Icon } from "../ui";
import {
  ChatContextFile,
  DiffChunk,
  BackgroundAgentSummary,
  isMultiModalToolResult,
  extractExecMetadata,
  MultiModalToolResult,
  ToolCall,
  ToolResult,
  ToolUsage,
} from "../../services/refact";
import { CommandMarkdown } from "../Command";
import { Chevron } from "../Collapsible";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectChatId,
  selectIsStreaming,
  selectIsWaiting,
  selectBackgroundAgentsByThread,
  selectManyDiffMessageByIds,
  selectManyToolResultsByIds,
  selectToolResultById,
} from "../../features/Chat/Thread/selectors";
import { ScrollArea } from "../ScrollArea";
import { takeWhile } from "../../utils";
import { DialogImage } from "../DialogImage";
import { RootState } from "../../app/store";
import { createChatWithId, switchToThread } from "../../features/Chat/Thread";
import { selectFeatures } from "../../features/Config/configSlice";
import { push } from "../../features/Pages/pagesSlice";
import { isRawTextDocToolCall } from "../Tools/types";
import {
  normalizeToolCall,
  normalizeToolName,
  formatToolDisplayName,
} from "../../utils/toolNameAliases";
import { useCollapsibleStore, useStoredOpen } from "./useStoredOpen";
import { AnimatedText } from "../Text";
import { toolCallArgsToString } from "./toolCallArgs";
import {
  ReadTool,
  ListTool,
  SearchTool,
  WebTool,
  KnowledgeTool,
  ShellTool as NewShellTool,
  ExecToolCard,
  SubagentTool as NewSubagentTool,
  PlanningTool,
  CodeReviewTool as NewCodeReviewTool,
  ResearchTool,
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
  ToolCard,
} from "./ToolCard";
import { AgentStatusView } from "./AgentStatusView";
import { AgentPulseView } from "./AgentPulseView";
import { AgentDiffView } from "./AgentDiffView";
import { TaskDocumentsView } from "./TaskDocumentsView";
import { FinalReportView } from "./FinalReportView";
import { BackgroundAgentCard } from "../BackgroundAgentCard";
import styles from "./ToolsContent.module.css";

function finalReportSuccess(content: string): boolean | null {
  try {
    const parsed = JSON.parse(content) as unknown;
    if (typeof parsed !== "object" || parsed === null) return null;
    if (!("success" in parsed)) return null;
    return typeof parsed.success === "boolean" ? parsed.success : null;
  } catch {
    return null;
  }
}

type FinalReportToolCardProps = {
  toolCall: ToolCall;
  content: string;
  toolFailed?: boolean;
};

const FinalReportToolCard: React.FC<FinalReportToolCardProps> = ({
  toolCall,
  content,
  toolFailed,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const reportSuccess = finalReportSuccess(content);
  const isError = Boolean(toolFailed) || reportSuccess === false;
  const status = isError ? "error" : "success";
  const statusIcon =
    status === "error" ? (
      <CircleX data-testid="final-report-tool-error-icon" />
    ) : (
      <CircleCheck data-testid="final-report-tool-success-icon" />
    );

  return (
    <>
      <span data-testid="final-report-tool" hidden />
      <ToolCard
        icon={statusIcon}
        summary="Task agent final report"
        meta={
          reportSuccess === false
            ? "failed"
            : reportSuccess === true
              ? "success"
              : undefined
        }
        status={status}
        isOpen={isOpen}
        onToggle={handleToggle}
        toolCall={toolCall}
      >
        <FinalReportView content={content} />
      </ToolCard>
    </>
  );
};

function parseProgressEntry(entry: string): { step?: string; text: string } {
  const m = entry.match(/^(\d+\/\d+):\s*([\s\S]+)$/);
  if (!m) return { text: entry };
  const [, step, text] = m;
  return { step, text };
}

const EXEC_TOOL_NAMES = new Set([
  "process_start",
  "process_list",
  "process_read",
  "process_kill",
  "process_wait",
  "process_write_stdin",
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

function backgroundAgentPlaceholder(
  result: ToolResult,
  toolName: "subagent" | "delegate",
): BackgroundAgentSummary | null {
  const agentId = getBackgroundAgentId(result);
  if (!agentId) return null;
  const kind = readBackgroundAgentKind(result, toolName);
  return {
    agent_id: agentId,
    parent_chat_id: readStringField(result, "parent_chat_id") ?? "",
    child_chat_id: readStringField(result, "child_chat_id"),
    kind,
    status: readBackgroundAgentStatus(result),
    title: readStringField(result, "title") ?? formatToolDisplayName(toolName),
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
  } satisfies BackgroundAgentSummary;
}

function decorateBackgroundAgentTool(
  elem: React.ReactNode,
  toolName: string | undefined,
  result: ToolResult | undefined,
  backgroundAgents: Partial<Record<string, BackgroundAgentSummary>>,
  onOpenTrajectory: (
    agent: BackgroundAgentSummary,
    childChatId: string,
  ) => void,
): React.ReactNode {
  if (!isBackgroundAgentTool(toolName)) return elem;
  const agentId = getBackgroundAgentId(result);
  if (!result || !agentId) return elem;
  const agent =
    backgroundAgents[agentId] ?? backgroundAgentPlaceholder(result, toolName);
  if (!agent) return elem;
  return (
    <React.Fragment key={`background-agent-${agent.agent_id}`}>
      {elem}
      <BackgroundAgentCard
        agent={agent}
        onOpenTrajectory={
          agent.child_chat_id
            ? (childChatId) => onOpenTrajectory(agent, childChatId)
            : undefined
        }
      />
    </React.Fragment>
  );
}

// TODO: Sort of duplicated
const ToolMessage: React.FC<{
  toolCall: ToolCall;
}> = ({ toolCall }) => {
  const name = normalizeToolName(toolCall.function.name) ?? "";
  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const argsString = React.useMemo(() => {
    return toolCallArgsToString(toolCall.function.arguments);
  }, [toolCall.function.arguments]);

  if (maybeResult && isMultiModalToolResult(maybeResult)) {
    // TODO: handle this
    return null;
  }

  const functionCalled = "```python\n" + name + "(" + argsString + ")\n```";

  return (
    <Flex direction="column">
      <ScrollArea scrollbars="horizontal" style={{ width: "100%" }}>
        <Box>
          <CommandMarkdown isInsideScrollArea>{functionCalled}</CommandMarkdown>
        </Box>
      </ScrollArea>
    </Flex>
  );
};

const ToolUsageDisplay: React.FC<{
  functionName: string;
  amountOfCalls: number;
}> = ({ functionName, amountOfCalls }) => {
  return (
    <>
      {formatToolDisplayName(functionName)}
      {amountOfCalls > 1 ? ` (${amountOfCalls})` : ""}
    </>
  );
};

const AnimatedCollapsibleContent: React.FC<{
  open: boolean;
  children: React.ReactNode;
}> = ({ open, children }) => {
  return (
    <Collapsible.Content forceMount asChild>
      <div
        className={classNames("rf-expand-grid", styles.toolGroupGrid)}
        data-open={open}
        hidden={false}
      >
        <div className={styles.toolGroupBody}>{children}</div>
      </div>
    </Collapsible.Content>
  );
};

// Use this for a single tool results
export const SingleModelToolContent: React.FC<{
  toolCalls: ToolCall[];
}> = ({ toolCalls }) => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const store = useCollapsibleStore();

  const toolCallsId = useMemo(() => {
    const out: string[] = [];
    for (const toolCall of toolCalls) {
      if (typeof toolCall.id === "string") {
        out.push(toolCall.id);
      }
    }
    return out;
  }, [toolCalls]);

  const toolCallsIdKey = toolCallsId.join("|");
  const storeKey = toolCallsId[0] ? `tg:${toolCallsId[0]}` : undefined;
  const [open, setOpen] = React.useState(() => {
    if (storeKey && store) {
      const stored = store.get(storeKey);
      if (stored !== undefined) return stored;
    }
    return false;
  });

  useEffect(() => {
    if (storeKey && store) store.set(storeKey, open);
  }, [storeKey, store, open]);
  const selectResults = useMemo(
    () => selectManyToolResultsByIds(toolCallsId),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [toolCallsIdKey],
  );
  const selectDiffs = useMemo(
    () => selectManyDiffMessageByIds(toolCallsId),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [toolCallsIdKey],
  );
  const results = useAppSelector(selectResults);
  const diffs = useAppSelector(selectDiffs);
  const allResolved = useMemo(() => {
    const resolvedToolIds = new Set<string>();
    for (const result of results) {
      resolvedToolIds.add(result.tool_call_id);
    }
    for (const diff of diffs) {
      resolvedToolIds.add(diff.tool_call_id);
    }
    return toolCallsId.every((id) => resolvedToolIds.has(id));
  }, [diffs, results, toolCallsId]);

  const busy = useMemo(() => {
    if (allResolved) return false;
    return isStreaming || isWaiting;
  }, [allResolved, isStreaming, isWaiting]);

  if (toolCalls.length === 0) return null;

  const toolNames = toolCalls.reduce<string[]>((acc, toolCall) => {
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    if (toolCall === null) {
      // eslint-disable-next-line no-console
      console.error("toolCall is null");
      return acc;
    }
    const normalizedName = normalizeToolName(toolCall.function.name);
    if (!normalizedName) return acc;
    if (acc.includes(normalizedName)) return acc;
    return [...acc, normalizedName];
  }, []);

  /*
    Calculates the usage amount of each tool by mapping over the unique tool names
    and counting how many times each tool has been called in the toolCalls array.
  */
  const toolUsageAmount = toolNames.map<ToolUsage>((toolName) => {
    return {
      functionName: toolName,
      amountOfCalls: toolCalls.filter(
        (toolCall) => normalizeToolName(toolCall.function.name) === toolName,
      ).length,
    };
  });

  const subchatLog: string[] = toolCalls.flatMap((tc) => tc.subchat_log ?? []);
  const attachedFiles = toolCalls
    .flatMap((tc) => tc.attached_files ?? [])
    .filter((f, i, arr) => arr.indexOf(f) === i);
  const shownAttachedFiles = attachedFiles.slice(-6);
  const hiddenFiles = Math.max(0, attachedFiles.length - 6);

  // Use this for single tool result
  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <ToolUsageSummary
            toolUsageAmount={toolUsageAmount}
            hiddenFiles={hiddenFiles}
            shownAttachedFiles={shownAttachedFiles}
            subchatLog={subchatLog}
            open={open}
            onClick={() => setOpen((prev) => !prev)}
            waiting={busy}
          />
        </Collapsible.Trigger>
        <AnimatedCollapsibleContent open={open}>
          {toolCalls.map((toolCall) => {
            // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
            if (toolCall === null) {
              // eslint-disable-next-line no-console
              console.error("toolCall is null");
              return;
            }
            if (toolCall.id === undefined) return;
            const key = `${toolCall.id}-${toolCall.index}`;
            return (
              <Box key={key} py="2">
                <ToolMessage toolCall={toolCall} />
              </Box>
            );
          })}
        </AnimatedCollapsibleContent>
      </Collapsible.Root>
    </Container>
  );
};

export type ToolContentProps = {
  toolCalls: ToolCall[];
  contextFilesByToolId?: Record<string, ChatContextFile[]>;
  diffsByToolId?: Record<string, DiffChunk[]>;
  isActiveAssistant?: boolean;
  threadId?: string;
};

export const ToolContent: React.FC<ToolContentProps> = ({
  toolCalls,
  contextFilesByToolId,
  diffsByToolId,
  isActiveAssistant = false,
  threadId,
}) => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const toolThreadId = threadId ?? chatId;
  const features = useAppSelector(selectFeatures);
  const backgroundAgents = useAppSelector((state) =>
    selectBackgroundAgentsByThread(state, chatId),
  );
  const handleOpenTrajectory = useCallback(
    (agent: BackgroundAgentSummary, childChatId: string) => {
      dispatch(
        createChatWithId({
          id: childChatId,
          title: agent.title,
          parentId: agent.parent_chat_id || chatId,
          linkType: agent.kind,
        }),
      );
      dispatch(switchToThread({ id: childChatId }));
      dispatch(push({ name: "chat" }));
    },
    [chatId, dispatch],
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
    () => selectManyToolResultsByIds(ids),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [idsKey],
  );
  const allToolResults = useAppSelector(selectResults);
  const activeToolCallId = isActiveAssistant ? ids[ids.length - 1] : undefined;

  return processToolCalls(
    toolCalls,
    allToolResults,
    features,
    [],
    contextFilesByToolId,
    diffsByToolId,
    activeToolCallId,
    backgroundAgents,
    handleOpenTrajectory,
    toolThreadId,
  );
};

function processToolCalls(
  toolCalls: ToolCall[],
  toolResults: ToolResult[],
  features: RootState["config"]["features"] = {},
  processed: React.ReactNode[] = [],
  contextFilesByToolId: Record<string, ChatContextFile[]> = {},
  diffsByToolId: Record<string, DiffChunk[]> = {},
  activeToolCallId?: string,
  backgroundAgents: Record<string, BackgroundAgentSummary> = {},
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "subagent") {
    const elem = (
      <NewSubagentTool
        key={`subagent-tool-${processed.length}`}
        toolCall={normalizedHead}
      />
    );
    const decoratedElem = decorateBackgroundAgentTool(
      elem,
      headName,
      result,
      backgroundAgents,
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "delegate") {
    const elem = (
      <GenericTool
        key={`delegate-tool-${head.id ?? processed.length}`}
        toolCall={normalizedHead}
      />
    );
    const decoratedElem = decorateBackgroundAgentTool(
      elem,
      headName,
      result,
      backgroundAgents,
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "strategic_planning") {
    const elem = (
      <PlanningTool
        key={`strategic-planning-tool-${head.id ?? processed.length}`}
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "code_review") {
    const elem = (
      <NewCodeReviewTool
        key={`code-review-tool-${head.id ?? processed.length}`}
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "deep_research") {
    const elem = (
      <ResearchTool
        key={`deep-research-tool-${head.id ?? processed.length}`}
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "agent_finish") {
    const elem =
      result && typeof result.content === "string" ? (
        <FinalReportToolCard
          key={`final-report-tool-${head.id ?? processed.length}`}
          toolCall={normalizedHead}
          content={result.content}
          toolFailed={result.tool_failed}
        />
      ) : (
        <GenericTool
          key={`final-report-tool-${head.id ?? processed.length}`}
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
      backgroundAgents,
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (headName === "ask_questions") {
    const elem = (
      <AskQuestionsTool
        key={`ask-questions-tool-${processed.length}`}
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
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
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  if (result && isMultiModalToolResult(result)) {
    const restInTail = takeWhile(tail, (toolCall) => {
      const nextResult = toolResults.find(
        (res) => res.tool_call_id === toolCall.id,
      );
      return nextResult !== undefined && isMultiModalToolResult(nextResult);
    });

    const nextTail = tail.slice(restInTail.length);
    const multiModalToolCalls = [
      normalizedHead,
      ...restInTail.map(normalizeToolCall),
    ];
    const ids = multiModalToolCalls.map((d) => d.id);
    const multiModalToolResults: MultiModalToolResult[] = toolResults
      .filter(isMultiModalToolResult)
      .filter((toolResult) => ids.includes(toolResult.tool_call_id));

    const elem = (
      <MultiModalToolContent
        key={`multi-model-tool-content-${processed.length}`}
        toolCalls={multiModalToolCalls}
        toolResults={multiModalToolResults}
      />
    );
    return processToolCalls(
      nextTail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
      activeToolCallId,
      backgroundAgents,
      onOpenTrajectory,
      threadId,
    );
  }

  // Fallback: use GenericTool for any unhandled tool
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
    backgroundAgents,
    onOpenTrajectory,
    threadId,
  );
}

const MultiModalToolContent: React.FC<{
  toolCalls: ToolCall[];
  toolResults: MultiModalToolResult[];
}> = ({ toolCalls, toolResults }) => {
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const store = useCollapsibleStore();

  const ids = useMemo(() => {
    return toolCalls
      .map((tc) => tc.id)
      .filter((id): id is string => typeof id === "string");
  }, [toolCalls]);

  const idsKey = ids.join("|");
  const mmStoreKey = ids[0] ? `mm:${ids[0]}` : undefined;
  const [open, setOpen] = React.useState(() => {
    if (mmStoreKey && store) {
      const stored = store.get(mmStoreKey);
      if (stored !== undefined) return stored;
    }
    return false;
  });

  useEffect(() => {
    if (mmStoreKey && store) store.set(mmStoreKey, open);
  }, [mmStoreKey, store, open]);

  const selectDiffs = useMemo(
    () => selectManyDiffMessageByIds(ids),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [idsKey],
  );
  const diffs = useAppSelector(selectDiffs);

  const hasImages = toolResults.some((toolResult) =>
    toolResult.content.some((content) => content.m_type.startsWith("image/")),
  );

  const toolNames = toolCalls.reduce<string[]>((acc, toolCall) => {
    const normalizedName = normalizeToolName(toolCall.function.name);
    if (!normalizedName) return acc;
    if (acc.includes(normalizedName)) return acc;
    return [...acc, normalizedName];
  }, []);

  const toolUsageAmount = toolNames.map<ToolUsage>((toolName) => {
    return {
      functionName: toolName,
      amountOfCalls: toolCalls.filter(
        (toolCall) => normalizeToolName(toolCall.function.name) === toolName,
      ).length,
    };
  });

  const hasResults = useMemo(() => {
    const diffIds = diffs.map((diff) => diff.tool_call_id);
    const toolIds = toolResults.map((d) => d.tool_call_id);
    const resultIds = [...diffIds, ...toolIds];
    return toolCalls.every(
      (toolCall) => toolCall.id && resultIds.includes(toolCall.id),
    );
  }, [toolCalls, toolResults, diffs]);

  const busy = useMemo(() => {
    if (hasResults) return false;
    return isStreaming || isWaiting;
  }, [hasResults, isStreaming, isWaiting]);

  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <ToolUsageSummary
            toolUsageAmount={toolUsageAmount}
            open={open}
            onClick={() => setOpen((prev) => !prev)}
            waiting={busy}
          />
        </Collapsible.Trigger>
        <AnimatedCollapsibleContent open={open}>
          <Box py="2">
            {toolCalls.map((toolCall, i) => {
              const result = toolResults.find(
                (toolResult) => toolResult.tool_call_id === toolCall.id,
              );
              if (!result) return null;

              const name = normalizeToolName(toolCall.function.name) ?? "";
              const argsString = toolCallArgsToString(
                toolCall.function.arguments,
              );
              const functionCalled =
                "```python\n" + name + "(" + argsString + ")\n```";

              return (
                <Flex
                  direction="column"
                  key={`tool-call-command-${toolCall.id}-${i}`}
                  py="2"
                >
                  <ScrollArea scrollbars="horizontal" style={{ width: "100%" }}>
                    <Box>
                      <CommandMarkdown isInsideScrollArea>
                        {functionCalled}
                      </CommandMarkdown>
                    </Box>
                  </ScrollArea>
                </Flex>
              );
            })}
          </Box>
        </AnimatedCollapsibleContent>
      </Collapsible.Root>
      {hasImages && (
        <Flex py="2" gap="2" wrap="wrap">
          {toolCalls.map((toolCall, index) => {
            const toolResult = toolResults.find(
              (toolResult) => toolResult.tool_call_id === toolCall.id,
            );
            if (!toolResult) return null;

            const images = toolResult.content.filter((content) =>
              content.m_type.startsWith("image/"),
            );
            if (images.length === 0) return null;

            return images.map((image, idx) => {
              const dataUrl = `data:${image.m_type};base64,${image.m_content}`;
              const key = `tool-image-${toolResult.tool_call_id}-${index}-${idx}`;
              return (
                <DialogImage key={key} size="8" src={dataUrl} fallback="" />
              );
            });
          })}
        </Flex>
      )}
    </Container>
  );
};

type ToolUsageSummaryProps = {
  toolUsageAmount: ToolUsage[];
  hiddenFiles?: number;
  shownAttachedFiles?: string[];
  subchatLog?: string[];
  open: boolean;
  onClick?: () => void;
  waiting: boolean;
};

function getFile(path: string): React.ReactNode {
  if (path.endsWith("/") || !path.includes(".")) return <Rows3 />;
  return <File />;
}

function truncatePath(path: string, maxLen = 50): string {
  if (path.length <= maxLen) return path;
  const parts = path.split("/");
  if (parts.length <= 2) return "…" + path.slice(-maxLen + 1);
  const filename = parts[parts.length - 1];
  const dir = parts[parts.length - 2];
  const short = `…/${dir}/${filename}`;
  if (short.length <= maxLen) return short;
  return "…" + path.slice(-maxLen + 1);
}

const ToolUsageSummary = forwardRef<HTMLDivElement, ToolUsageSummaryProps>(
  (
    {
      toolUsageAmount,
      hiddenFiles,
      shownAttachedFiles,
      subchatLog,
      open,
      onClick,
      waiting,
    },
    ref,
  ) => {
    const currentStep = (subchatLog ?? []).slice(-1)[0];

    return (
      <AnimatedText
        as="div"
        weight="light"
        size="1"
        animating={waiting}
        className={styles.toolUsageSummary}
      >
        <Flex gap="2" align="end" onClick={onClick} ref={ref} my="2">
          <Flex
            gap="1"
            align="start"
            direction="column"
            className={styles.toolUsageSummaryBody}
          >
            <Flex gap="2" align="center" justify="center">
              {waiting ? (
                <Icon icon={LoaderCircle} size="sm" tone="accent" />
              ) : (
                <Settings />
              )}
              {toolUsageAmount.map(({ functionName, amountOfCalls }, index) => (
                <span key={functionName}>
                  <ToolUsageDisplay
                    functionName={functionName}
                    amountOfCalls={amountOfCalls}
                  />
                  {index === toolUsageAmount.length - 1 ? "" : ", "}
                </span>
              ))}
            </Flex>

            {hiddenFiles !== undefined && hiddenFiles > 0 && (
              <Text weight="light" size="1" ml="4">
                {`<+${hiddenFiles} more files>`}
              </Text>
            )}
            {shownAttachedFiles?.map((file, index) => (
              <Text weight="light" size="1" key={index} ml="4" as="div">
                {getFile(file)} {truncatePath(file)}
              </Text>
            ))}
            {currentStep &&
              (() => {
                const parsed = parseProgressEntry(currentStep);
                return (
                  <Flex direction="column" gap="1" ml="4" mt="1">
                    {parsed.step && (
                      <Flex align="center" gap="1">
                        {waiting && (
                          <Icon icon={LoaderCircle} size="sm" tone="accent" />
                        )}
                        <Text weight="light" size="1">
                          {parsed.step}:
                        </Text>
                      </Flex>
                    )}
                    <Text
                      size="1"
                      color="gray"
                      as="div"
                      ml={parsed.step ? "4" : "0"}
                      style={{
                        whiteSpace: "pre-wrap",
                        wordBreak: "break-word",
                      }}
                    >
                      {parsed.text}
                    </Text>
                  </Flex>
                );
              })()}
          </Flex>
          <Chevron open={open} />
        </Flex>
      </AnimatedText>
    );
  },
);
ToolUsageSummary.displayName = "ToolUsageSummary";
