import React, { forwardRef, useCallback, useMemo, useRef } from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import { Container, Flex, Text, Box, Spinner } from "@radix-ui/themes";
import {
  ChatContextFile,
  DiffChunk,
  isMultiModalToolResult,
  MultiModalToolResult,
  ToolCall,
  ToolResult,
  ToolUsage,
} from "../../services/refact";
import styles from "./ChatContent.module.css";
import { CommandMarkdown } from "../Command";
import { Chevron } from "../Collapsible";
import { Reveal } from "../Reveal";
import { useAppSelector, useHideScroll } from "../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectManyDiffMessageByIds,
  selectManyToolResultsByIds,
  selectToolResultById,
} from "../../features/Chat/Thread/selectors";
import { ScrollArea } from "../ScrollArea";
import { takeWhile } from "../../utils";
import { DialogImage } from "../DialogImage";
import { RootState } from "../../app/store";
import { selectFeatures } from "../../features/Config/configSlice";
import { isRawTextDocToolCall } from "../Tools/types";
import { ShikiCodeBlock } from "../Markdown/ShikiCodeBlock";
import { Markdown } from "../Markdown";
import classNames from "classnames";
import { AnimatedText } from "../Text";
import {
  ReadTool,
  ListTool,
  SearchTool,
  WebTool,
  KnowledgeTool,
  ShellTool as NewShellTool,
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
} from "./ToolCard";

function parseProgressEntry(entry: string): { step?: string; lines: string[] } {
  const m = entry.match(/^(\d+\/\d+): ([\s\S]+)$/);
  if (!m) return { lines: [entry] };
  const [, step, content] = m;
  return { step, lines: content.split("\n").filter((l) => l.trim()) };
}

type ResultProps = {
  children: string;
  isInsideScrollArea?: boolean;
  onClose?: () => void;
};

function looksLikeMarkdown(text: string): boolean {
  // Strong signals to avoid false positives on logs/stack traces
  if (text.includes("```")) return true; // fenced code blocks
  if (/\[[^\]]+\]\([^)]+\)/.test(text)) return true; // [text](url)
  if (/^#{1,6}\s+\S/m.test(text)) return true; // headings
  if (/^\s*([-*+])\s+\S/m.test(text)) return true; // unordered lists
  if (/^\s*\d+\.\s+\S/m.test(text)) return true; // ordered lists

  // Table detection: header row + separator row
  const hasTableHeader = /^\s*\|.+\|\s*$/m.test(text);
  const hasTableSep = /^\s*\|[\s:|-]+\|\s*$/m.test(text);
  if (hasTableHeader && hasTableSep) return true;

  return false;
}

const MAX_MD_RENDER_CHARS = 50_000;

const Result: React.FC<ResultProps> = ({ children, onClose }) => {
  const lines = children.split("\n");

  const shouldRenderMarkdown =
    children.length <= MAX_MD_RENDER_CHARS && looksLikeMarkdown(children);

  return (
    <Reveal defaultOpen={lines.length < 9} isRevealingCode onClose={onClose}>
      {shouldRenderMarkdown ? (
        <Text size="2">
          <Box
            className={classNames(
              styles.tool_result,
              styles.tool_result_markdown,
            )}
          >
            <Markdown>{children}</Markdown>
          </Box>
        </Text>
      ) : (
        <ShikiCodeBlock className={classNames(styles.tool_result)}>
          {children}
        </ShikiCodeBlock>
      )}
    </Reveal>
  );
};

function toolCallArgsToString(toolCallArgs: string) {
  try {
    const json = JSON.parse(toolCallArgs) as unknown as Parameters<
      typeof Object.entries
    >;
    if (Array.isArray(json)) {
      return json.join(", ");
    }
    return Object.entries(json)
      .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
      .join(", ");
  } catch {
    return toolCallArgs;
  }
}

// TODO: Sort of duplicated
const ToolMessage: React.FC<{
  toolCall: ToolCall;
  onClose: () => void;
}> = ({ toolCall, onClose }) => {
  const name = toolCall.function.name ?? "";
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
      {maybeResult?.content && (
        <Result isInsideScrollArea onClose={onClose}>
          {maybeResult.content}
        </Result>
      )}
    </Flex>
  );
};

const ToolUsageDisplay: React.FC<{
  functionName: string;
  amountOfCalls: number;
}> = ({ functionName, amountOfCalls }) => {
  return (
    <>
      {functionName}
      {amountOfCalls > 1 ? ` (${amountOfCalls})` : ""}
    </>
  );
};

// Use this for a single tool results
export const SingleModelToolContent: React.FC<{
  toolCalls: ToolCall[];
}> = ({ toolCalls }) => {
  const [open, setOpen] = React.useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const handleHide = useHideScroll(ref);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const toolCallsId = useMemo(() => {
    const ids = toolCalls.reduce<string[]>((acc, toolCall) => {
      if (typeof toolCall.id === "string") return [...acc, toolCall.id];
      return acc;
    }, []);

    return ids;
  }, [toolCalls]);

  const results = useAppSelector(selectManyToolResultsByIds(toolCallsId));
  const diffs = useAppSelector(selectManyDiffMessageByIds(toolCallsId));
  const allResolved = useMemo(() => {
    return results.length + diffs.length === toolCallsId.length;
  }, [diffs.length, results.length, toolCallsId.length]);

  const busy = useMemo(() => {
    if (allResolved) return false;
    return isStreaming || isWaiting;
  }, [allResolved, isStreaming, isWaiting]);

  const handleClose = useCallback(() => {
    handleHide();
    setOpen(false);
  }, [handleHide]);

  if (toolCalls.length === 0) return null;

  const toolNames = toolCalls.reduce<string[]>((acc, toolCall) => {
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    if (toolCall === null) {
      // eslint-disable-next-line no-console
      console.error("toolCall is null");
      return acc;
    }
    if (!toolCall.function.name) return acc;
    if (acc.includes(toolCall.function.name)) return acc;
    return [...acc, toolCall.function.name];
  }, []);

  /*
    Calculates the usage amount of each tool by mapping over the unique tool names
    and counting how many times each tool has been called in the toolCalls array.
  */
  const toolUsageAmount = toolNames.map<ToolUsage>((toolName) => {
    return {
      functionName: toolName,
      amountOfCalls: toolCalls.filter(
        (toolCall) => toolCall.function.name === toolName,
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
            ref={ref}
            toolUsageAmount={toolUsageAmount}
            hiddenFiles={hiddenFiles}
            shownAttachedFiles={shownAttachedFiles}
            subchatLog={subchatLog}
            open={open}
            onClick={() => setOpen((prev) => !prev)}
            waiting={busy}
          />
        </Collapsible.Trigger>
        <Collapsible.Content>
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
                <ToolMessage toolCall={toolCall} onClose={handleClose} />
              </Box>
            );
          })}
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};

export type ToolContentProps = {
  toolCalls: ToolCall[];
  contextFilesByToolId?: Record<string, ChatContextFile[]>;
  diffsByToolId?: Record<string, DiffChunk[]>;
};

export const ToolContent: React.FC<ToolContentProps> = ({
  toolCalls,
  contextFilesByToolId,
  diffsByToolId,
}) => {
  const features = useAppSelector(selectFeatures);
  const ids = toolCalls.reduce<string[]>((acc, cur) => {
    if (cur.id !== undefined) return [...acc, cur.id];
    return acc;
  }, []);
  const allToolResults = useAppSelector(selectManyToolResultsByIds(ids));

  return processToolCalls(
    toolCalls,
    allToolResults,
    features,
    [],
    contextFilesByToolId,
    diffsByToolId,
  );
};

function processToolCalls(
  toolCalls: ToolCall[],
  toolResults: ToolResult[],
  features: RootState["config"]["features"] = {},
  processed: React.ReactNode[] = [],
  contextFilesByToolId: Record<string, ChatContextFile[]> = {},
  diffsByToolId: Record<string, DiffChunk[]> = {},
) {
  if (toolCalls.length === 0) return processed;
  const [head, ...tail] = toolCalls;
  const result = toolResults.find((result) => result.tool_call_id === head.id);
  const contextFiles = head.id ? contextFilesByToolId[head.id] : undefined;
  const diffs = head.id ? diffsByToolId[head.id] : undefined;

  if (head.function.name === "cat") {
    const elem = (
      <ReadTool
        key={`read-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "tree") {
    const elem = (
      <ListTool
        key={`list-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "search_pattern") {
    const elem = (
      <SearchTool
        key={`search-pattern-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "search_semantic") {
    const elem = (
      <SearchTool
        key={`search-semantic-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "search_symbol_definition") {
    const elem = (
      <SearchTool
        key={`search-symbol-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "shell") {
    const elem = (
      <NewShellTool key={`shell-tool-${processed.length}`} toolCall={head} />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "shell_service") {
    const elem = (
      <NewShellServiceTool
        key={`shell-service-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "subagent") {
    const elem = (
      <NewSubagentTool
        key={`subagent-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "strategic_planning") {
    const elem = (
      <PlanningTool
        key={`strategic-planning-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "code_review") {
    const elem = (
      <NewCodeReviewTool
        key={`code-review-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "deep_research") {
    const elem = (
      <ResearchTool
        key={`deep-research-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "knowledge") {
    const elem = (
      <KnowledgeTool
        key={`knowledge-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "search_trajectories") {
    const elem = (
      <KnowledgeTool
        key={`trajectories-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "get_trajectory_context") {
    const elem = (
      <KnowledgeTool
        key={`trajectory-context-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "create_knowledge") {
    const elem = (
      <KnowledgeTool
        key={`create-knowledge-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "web") {
    const elem = (
      <WebTool
        key={`web-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (head.function.name === "web_search") {
    const elem = (
      <WebTool
        key={`web-search-tool-${processed.length}`}
        toolCall={head}
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
    );
  }

  if (isRawTextDocToolCall(head)) {
    const elem = (
      <EditTool
        key={`edit-tool-${head.function.name}-${processed.length}`}
        toolCall={head}
        diffs={diffs}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "mv") {
    const elem = (
      <FileOpTool
        key={`mv-tool-${processed.length}`}
        toolCall={head}
        toolType="mv"
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "rm") {
    const elem = (
      <FileOpTool
        key={`rm-tool-${processed.length}`}
        toolCall={head}
        toolType="rm"
        diffs={diffs}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "add_workspace_folder") {
    const elem = (
      <FileOpTool
        key={`add-workspace-tool-${processed.length}`}
        toolCall={head}
        toolType="add_workspace_folder"
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "tasks_set") {
    const elem = (
      <TasksTool key={`tasks-tool-${processed.length}`} toolCall={head} />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "task_done") {
    const elem = (
      <TaskDoneTool
        key={`task-done-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
    );
  }

  if (head.function.name === "ask_questions") {
    const elem = (
      <AskQuestionsTool
        key={`ask-questions-tool-${processed.length}`}
        toolCall={head}
      />
    );
    return processToolCalls(
      tail,
      toolResults,
      features,
      [...processed, elem],
      contextFilesByToolId,
      diffsByToolId,
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
    const multiModalToolCalls = [head, ...restInTail];
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
    );
  }

  // Fallback: use GenericTool for any unhandled tool
  const elem = (
    <GenericTool
      key={`generic-tool-${head.id ?? processed.length}`}
      toolCall={head}
    />
  );
  return processToolCalls(
    tail,
    toolResults,
    features,
    [...processed, elem],
    contextFilesByToolId,
    diffsByToolId,
  );
}

const MultiModalToolContent: React.FC<{
  toolCalls: ToolCall[];
  toolResults: MultiModalToolResult[];
}> = ({ toolCalls, toolResults }) => {
  const [open, setOpen] = React.useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const handleHide = useHideScroll(ref);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const ids = useMemo(() => {
    return toolCalls
      .map((tc) => tc.id)
      .filter((id): id is string => typeof id === "string");
  }, [toolCalls]);

  const diffs = useAppSelector(selectManyDiffMessageByIds(ids));

  const handleClose = useCallback(() => {
    handleHide();
    setOpen(false);
  }, [handleHide]);

  const hasImages = toolResults.some((toolResult) =>
    toolResult.content.some((content) => content.m_type.startsWith("image/")),
  );

  const toolNames = toolCalls.reduce<string[]>((acc, toolCall) => {
    if (!toolCall.function.name) return acc;
    if (acc.includes(toolCall.function.name)) return acc;
    return [...acc, toolCall.function.name];
  }, []);

  const toolUsageAmount = toolNames.map<ToolUsage>((toolName) => {
    return {
      functionName: toolName,
      amountOfCalls: toolCalls.filter(
        (toolCall) => toolCall.function.name === toolName,
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
            ref={ref}
            waiting={busy}
          />
        </Collapsible.Trigger>
        <Collapsible.Content>
          {/** TODO: tool call name and text result */}
          <Box py="2">
            {toolCalls.map((toolCall, i) => {
              const result = toolResults.find(
                (toolResult) => toolResult.tool_call_id === toolCall.id,
              );
              if (!result) return null;

              const texts = result.content
                .filter((content) => content.m_type === "text")
                .map((result) => result.m_content)
                .join("\n");

              const name = toolCall.function.name ?? "";
              const argsString = toolCallArgsToString(
                toolCall.function.arguments,
              );

              const functionCalled =
                "```python\n" + name + "(" + argsString + ")\n```";

              // TODO: sort of duplicated
              return (
                <Flex
                  direction="column"
                  key={`tool-call-command-${toolCall.id}-${i}`}
                  py="2"
                  ref={ref}
                >
                  <ScrollArea scrollbars="horizontal" style={{ width: "100%" }}>
                    <Box>
                      <CommandMarkdown isInsideScrollArea>
                        {functionCalled}
                      </CommandMarkdown>
                    </Box>
                  </ScrollArea>
                  <Box>
                    <Result onClose={handleClose}>{texts}</Result>
                  </Box>
                </Flex>
              );
            })}
          </Box>
        </Collapsible.Content>
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

function getFileIcon(path: string): string {
  if (path.endsWith("/") || !path.includes(".")) return "📂";
  return "📄";
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
      <AnimatedText as="div" weight="light" size="1" animating={waiting}>
        <Flex gap="2" align="end" onClick={onClick} ref={ref} my="2">
          <Flex
            gap="1"
            align="start"
            direction="column"
            style={{ cursor: "pointer" }}
          >
            <Flex gap="2" align="center" justify="center">
              {waiting ? <Spinner /> : "🔨"}
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
              <Text weight="light" size="1" key={index} ml="4">
                {getFileIcon(file)} {truncatePath(file)}
              </Text>
            ))}
            {currentStep &&
              (() => {
                const parsed = parseProgressEntry(currentStep);
                return (
                  <Flex direction="column" gap="1" ml="4" mt="1">
                    {parsed.step && (
                      <Flex align="center" gap="1">
                        {waiting && <Spinner size="1" />}
                        <Text weight="light" size="1">
                          {parsed.step}:
                        </Text>
                      </Flex>
                    )}
                    {parsed.lines.map((line, i) => (
                      <Text
                        key={i}
                        weight="light"
                        size="1"
                        ml={parsed.step ? "4" : "0"}
                      >
                        {parsed.step ? "🔨 " : ""}
                        {line}
                      </Text>
                    ))}
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
