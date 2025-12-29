import React, { forwardRef, useCallback, useMemo, useRef } from "react";
import * as Collapsible from "@radix-ui/react-collapsible";
import {
  Container,
  Flex,
  Text,
  Box,
  Spinner,
  Card,
  Separator,
} from "@radix-ui/themes";
import {
  isMultiModalToolResult,
  // knowledgeApi,
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
import { TextDocTool } from "../Tools/Textdoc";
import { MarkdownCodeBlock } from "../Markdown/CodeBlock";
import { Markdown } from "../Markdown";
import classNames from "classnames";
import resultStyle from "react-syntax-highlighter/dist/esm/styles/hljs/arta";
import { FadedButton } from "../Buttons";
import { AnimatedText } from "../Text";

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
            <Markdown style={resultStyle}>{children}</Markdown>
          </Box>
        </Text>
      ) : (
        <MarkdownCodeBlock
          className={classNames(styles.tool_result)}
          style={resultStyle}
        >
          {children}
        </MarkdownCodeBlock>
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

  const subchat: string | undefined = toolCalls
    .map((toolCall) => toolCall.subchat)
    .filter((x) => x)[0];
  const attachedFiles = toolCalls
    .map((toolCall) => toolCall.attached_files)
    .filter((x) => x)
    .flat();
  const shownAttachedFiles = attachedFiles.slice(-4);
  const hiddenFiles = attachedFiles.length - 4;

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
            subchat={subchat}
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
};

export const ToolContent: React.FC<ToolContentProps> = ({ toolCalls }) => {
  const features = useAppSelector(selectFeatures);
  const ids = toolCalls.reduce<string[]>((acc, cur) => {
    if (cur.id !== undefined) return [...acc, cur.id];
    return acc;
  }, []);
  const allToolResults = useAppSelector(selectManyToolResultsByIds(ids));

  return processToolCalls(toolCalls, allToolResults, features);
};

function processToolCalls(
  toolCalls: ToolCall[],
  toolResults: ToolResult[],
  features: RootState["config"]["features"] = {},
  processed: React.ReactNode[] = [],
) {
  if (toolCalls.length === 0) return processed;
  const [head, ...tail] = toolCalls;
  const result = toolResults.find((result) => result.tool_call_id === head.id);

  // TODO: handle knowledge differently.
  // memories are split in content with 🗃️019957b6ff

  if (result && head.function.name === "knowledge") {
    const elem = (
      <Knowledge key={`knowledge-tool-${processed.length}`} toolCall={head} />
    );
    return processToolCalls(tail, toolResults, features, [...processed, elem]);
  }

  if (result && head.function.name === "search_trajectories") {
    const elem = (
      <Trajectories key={`trajectories-tool-${processed.length}`} toolCall={head} />
    );
    return processToolCalls(tail, toolResults, features, [...processed, elem]);
  }

  if (result && head.function.name === "get_trajectory_context") {
    const elem = (
      <TrajectoryContext key={`trajectory-context-tool-${processed.length}`} toolCall={head} />
    );
    return processToolCalls(tail, toolResults, features, [...processed, elem]);
  }

  if (isRawTextDocToolCall(head)) {
    const elem = (
      <TextDocTool
        key={`textdoc-tool-${head.function.name}-${processed.length}`}
        toolCall={head}
        toolFailed={result?.tool_failed}
      />
    );
    return processToolCalls(tail, toolResults, features, [...processed, elem]);
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
    return processToolCalls(nextTail, toolResults, features, [
      ...processed,
      elem,
    ]);
  }

  const restInTail = takeWhile(tail, (toolCall) => {
    const item = toolResults.find(
      (result) => result.tool_call_id === toolCall.id,
    );
    return item === undefined || !isMultiModalToolResult(item);
  });
  const nextTail = tail.slice(restInTail.length);

  const elem = (
    <SingleModelToolContent
      key={`single-model-tool-call-${processed.length}`}
      toolCalls={[head, ...restInTail]}
    />
  );
  return processToolCalls(nextTail, toolResults, features, [
    ...processed,
    elem,
  ]);
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
  shownAttachedFiles?: (string | undefined)[];
  subchat?: string;
  open: boolean;
  onClick?: () => void;
  waiting: boolean;
};

const ToolUsageSummary = forwardRef<HTMLDivElement, ToolUsageSummaryProps>(
  (
    {
      toolUsageAmount,
      hiddenFiles,
      shownAttachedFiles,
      subchat,
      open,
      onClick,
      waiting,
    },
    ref,
  ) => {
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
              {waiting ? <Spinner /> : "🔨"} {/* 🔨{" "} */}
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

            {hiddenFiles && hiddenFiles > 0 && (
              <Text weight="light" size="1" ml="4">
                {`🔎 <${hiddenFiles} files hidden>`}
              </Text>
            )}
            {shownAttachedFiles?.map((file, index) => {
              if (!file) return null;

              return (
                <Text weight="light" size="1" key={index} ml="4">
                  🔎 {file}
                </Text>
              );
            })}
            {subchat && (
              <Flex ml="4">
                {waiting && <Spinner />}
                <Text weight="light" size="1" ml="4px">
                  {subchat}
                </Text>
              </Flex>
            )}
          </Flex>
          <Chevron open={open} />
        </Flex>
      </AnimatedText>
    );
  },
);
ToolUsageSummary.displayName = "ToolUsageSummary";

// TODO: make this look nicer.
const Knowledge: React.FC<{ toolCall: ToolCall }> = ({ toolCall }) => {
  const [open, setOpen] = React.useState(false);
  const ref = useRef(null);
  const scrollOnHide = useHideScroll(ref);

  const handleHide = useCallback(() => {
    setOpen(false);
    scrollOnHide();
  }, [scrollOnHide]);

  const name = toolCall.function.name ?? "";

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const argsString = React.useMemo(() => {
    return toolCallArgsToString(toolCall.function.arguments);
  }, [toolCall.function.arguments]);

  const memories = useMemo(() => {
    if (typeof maybeResult?.content !== "string") return [];
    return splitMemories(maybeResult.content);
  }, [maybeResult?.content]);

  const functionCalled = "```python\n" + name + "(" + argsString + ")\n```";

  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex
            gap="2"
            align="end"
            onClick={() => setOpen((prev) => !prev)}
            ref={ref}
          >
            <Flex
              gap="1"
              align="start"
              direction="column"
              style={{ cursor: "pointer" }}
            >
              <Text weight="light" size="1">
                📚 Knowledge
              </Text>
            </Flex>
            <Chevron open={open} />
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <Flex direction="column" pt="4">
            <ScrollArea scrollbars="horizontal" style={{ width: "100%" }}>
              <Box>
                <CommandMarkdown isInsideScrollArea>
                  {functionCalled}
                </CommandMarkdown>
              </Box>
            </ScrollArea>
            <Flex gap="4" direction="column" py="4">
              {memories.map((memory, idx) => (
                <Memory key={memory.title + idx} memory={memory} />
              ))}
            </Flex>
            <FadedButton color="gray" onClick={handleHide} mx="2">
              Hide Memories
            </FadedButton>
          </Flex>
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};

interface MemoryEntry {
  title: string;
  content: string;
}

const Memory: React.FC<{ memory: MemoryEntry }> = ({ memory }) => {
  return (
    <Card>
      <Flex direction="column" gap="2">
        <Flex justify="between" align="center">
          <Text size="1" weight="light">
            Memory: {memory.title}
          </Text>
        </Flex>
        <Separator size="4" />
        <Text size="2" style={{ whiteSpace: "pre-wrap" }}>{memory.content}</Text>
      </Flex>
    </Card>
  );
};

function splitMemories(text: string): MemoryEntry[] {
  const entries = text.split("\n\n---\n").filter((part) => part.trim());

  return entries.map((entry) => {
    const lines = entry.split("\n");
    let path = "";
    let title = "";
    const contentLines: string[] = [];

    for (const line of lines) {
      if (line.startsWith("📄 ")) {
        path = line.substring(3);
      } else if (line.startsWith("📌 ")) {
        title = line.substring(3);
      } else if (line.startsWith("📦 ") || line.startsWith("🏷️ ")) {
        continue;
      } else {
        contentLines.push(line);
      }
    }

    const displayTitle = title || extractReadableName(path);

    return {
      title: displayTitle,
      content: contentLines.join("\n").trim(),
    };
  });
}

function extractReadableName(path: string): string {
  const fileName = path.split("/").pop() ?? path;
  const memoryMatch = fileName.match(/^\d{4}-\d{2}-\d{2}_\d{6}_[a-f0-9]+_(.+)\.md$/);
  if (memoryMatch) {
    return memoryMatch[1].replace(/-/g, " ");
  }
  return fileName;
}

const Trajectories: React.FC<{ toolCall: ToolCall }> = ({ toolCall }) => {
  const [open, setOpen] = React.useState(false);
  const ref = useRef(null);
  const scrollOnHide = useHideScroll(ref);

  const handleHide = useCallback(() => {
    setOpen(false);
    scrollOnHide();
  }, [scrollOnHide]);

  const name = toolCall.function.name ?? "";

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const argsString = React.useMemo(() => {
    return toolCallArgsToString(toolCall.function.arguments);
  }, [toolCall.function.arguments]);

  const trajectories = useMemo(() => {
    if (typeof maybeResult?.content !== "string") return [];
    return splitTrajectories(maybeResult.content);
  }, [maybeResult?.content]);

  const functionCalled = "```python\n" + name + "(" + argsString + ")\n```";

  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex
            gap="2"
            align="end"
            onClick={() => setOpen((prev) => !prev)}
            ref={ref}
          >
            <Flex
              gap="1"
              align="start"
              direction="column"
              style={{ cursor: "pointer" }}
            >
              <Text weight="light" size="1">
                🕐 Past Conversations ({trajectories.length})
              </Text>
            </Flex>
            <Chevron open={open} />
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <Flex direction="column" pt="4">
            <ScrollArea scrollbars="horizontal" style={{ width: "100%" }}>
              <Box>
                <CommandMarkdown isInsideScrollArea>
                  {functionCalled}
                </CommandMarkdown>
              </Box>
            </ScrollArea>
            <Flex gap="4" direction="column" py="4">
              {trajectories.map((traj) => (
                <TrajectoryCard
                  key={traj.id}
                  id={traj.id}
                  title={traj.title}
                  relevance={traj.relevance}
                  messageRange={traj.messageRange}
                  preview={traj.preview}
                />
              ))}
            </Flex>
            <FadedButton color="gray" onClick={handleHide} mx="2">
              Hide Results
            </FadedButton>
          </Flex>
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};

const TrajectoryCard: React.FC<{
  id: string;
  title: string;
  relevance: string;
  messageRange: string;
  preview: string;
}> = ({ id, title, relevance, messageRange, preview }) => {
  return (
    <Card>
      <Flex direction="column" gap="2">
        <Flex justify="between" align="center">
          <Text size="1" weight="medium">
            📁 {id}
          </Text>
          {relevance && (
            <Text size="1" style={{ color: "var(--accent-9)" }}>
              {relevance}
            </Text>
          )}
        </Flex>
        {title && (
          <Text size="2" weight="medium">
            📌 {title}
          </Text>
        )}
        {messageRange && (
          <Text size="1" color="gray">
            📍 Messages: {messageRange}
          </Text>
        )}
        {preview && (
          <>
            <Separator size="4" />
            <Text size="1" style={{ whiteSpace: "pre-wrap", opacity: 0.8 }}>
              {preview}
            </Text>
          </>
        )}
      </Flex>
    </Card>
  );
};

const TrajectoryContext: React.FC<{ toolCall: ToolCall }> = ({ toolCall }) => {
  const [open, setOpen] = React.useState(false);
  const ref = useRef(null);
  const scrollOnHide = useHideScroll(ref);

  const handleHide = useCallback(() => {
    setOpen(false);
    scrollOnHide();
  }, [scrollOnHide]);

  const name = toolCall.function.name ?? "";

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const argsString = React.useMemo(() => {
    return toolCallArgsToString(toolCall.function.arguments);
  }, [toolCall.function.arguments]);

  const { header, messages } = useMemo(() => {
    if (typeof maybeResult?.content !== "string") return { header: null, messages: [] };
    return parseTrajectoryContext(maybeResult.content);
  }, [maybeResult?.content]);

  const functionCalled = "```python\n" + name + "(" + argsString + ")\n```";

  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex
            gap="2"
            align="end"
            onClick={() => setOpen((prev) => !prev)}
            ref={ref}
          >
            <Flex
              gap="1"
              align="start"
              direction="column"
              style={{ cursor: "pointer" }}
            >
              <Text weight="light" size="1">
                📜 Trajectory Context {header?.title && `- ${header.title}`}
              </Text>
            </Flex>
            <Chevron open={open} />
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <Flex direction="column" pt="4">
            <ScrollArea scrollbars="horizontal" style={{ width: "100%" }}>
              <Box>
                <CommandMarkdown isInsideScrollArea>
                  {functionCalled}
                </CommandMarkdown>
              </Box>
            </ScrollArea>
            {header && (
              <Card my="2">
                <Flex direction="column" gap="1">
                  <Text size="1" weight="medium">📁 {header.id}</Text>
                  <Text size="2" weight="medium">{header.title}</Text>
                  <Text size="1" color="gray">{header.range}</Text>
                </Flex>
              </Card>
            )}
            <Flex gap="3" direction="column" py="2">
              {messages.map((msg, idx) => (
                <Card key={idx} style={{ borderLeft: msg.highlighted ? "3px solid var(--accent-9)" : undefined }}>
                  <Flex direction="column" gap="1">
                    <Flex gap="2" align="center">
                      <Text size="1">{msg.icon}</Text>
                      <Text size="1" weight="medium" color={msg.highlighted ? "blue" : undefined}>
                        [{msg.index}] {msg.role}
                      </Text>
                    </Flex>
                    <Text size="2" style={{ whiteSpace: "pre-wrap" }}>{msg.content}</Text>
                  </Flex>
                </Card>
              ))}
            </Flex>
            <FadedButton color="gray" onClick={handleHide} mx="2">
              Hide Context
            </FadedButton>
          </Flex>
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};

interface TrajectoryHeader {
  id: string;
  title: string;
  range: string;
}

interface TrajectoryMessage {
  index: string;
  role: string;
  icon: string;
  content: string;
  highlighted: boolean;
}

function parseTrajectoryContext(text: string): { header: TrajectoryHeader | null; messages: TrajectoryMessage[] } {
  const lines = text.split("\n");
  let header: TrajectoryHeader | null = null;
  const messages: TrajectoryMessage[] = [];
  let currentMsg: TrajectoryMessage | null = null;
  let contentLines: string[] = [];

  for (const line of lines) {
    if (line.startsWith("│ 📁 ")) {
      if (!header) header = { id: "", title: "", range: "" };
      header.id = line.substring(5).replace(/│$/, "").trim();
    } else if (line.startsWith("│ 📌 ")) {
      if (!header) header = { id: "", title: "", range: "" };
      header.title = line.substring(5).replace(/│$/, "").trim();
    } else if (line.startsWith("│ 📍 ")) {
      if (!header) header = { id: "", title: "", range: "" };
      header.range = line.substring(5).replace(/│$/, "").trim();
    } else if (line.startsWith("┏━") || line.startsWith("┌─")) {
      if (currentMsg) {
        currentMsg.content = contentLines.join("\n").trim();
        messages.push(currentMsg);
        contentLines = [];
      }
      const highlighted = line.startsWith("┏━");
      const match = line.match(/([👤🤖🔧💬]) \[(\d+)\] (\w+)/u);
      if (match) {
        currentMsg = {
          icon: match[1],
          index: match[2],
          role: match[3],
          content: "",
          highlighted,
        };
      }
    } else if (currentMsg && !line.startsWith("╭") && !line.startsWith("╰") && !line.startsWith("│")) {
      contentLines.push(line);
    }
  }

  if (currentMsg) {
    currentMsg.content = contentLines.join("\n").trim();
    messages.push(currentMsg);
  }

  return { header, messages };
}

interface ParsedTrajectory {
  id: string;
  title: string;
  relevance: string;
  messageRange: string;
  preview: string;
}

function splitTrajectories(text: string): ParsedTrajectory[] {
  const entries = text.split("───────────────────────────────────────\n").filter((part) => part.trim() && part.includes("📁"));

  return entries.map((entry) => {
    const lines = entry.split("\n");
    let id = "";
    let title = "";
    let relevance = "";
    let messageRange = "";
    const previewLines: string[] = [];
    let inPreview = false;

    for (const line of lines) {
      if (line.startsWith("📁 ")) {
        id = line.substring(3).trim();
        inPreview = false;
      } else if (line.startsWith("📌 ")) {
        title = line.substring(3).trim();
        inPreview = false;
      } else if (line.startsWith("⭐ Relevance: ")) {
        relevance = line.substring(14).trim();
        inPreview = false;
      } else if (line.startsWith("📍 Messages: ")) {
        messageRange = line.substring(13).trim();
        inPreview = true;
      } else if (inPreview && line.trim() && !line.startsWith("💡")) {
        previewLines.push(line);
      }
    }

    return { id, title, relevance, messageRange, preview: previewLines.join("\n").trim() };
  });
}
