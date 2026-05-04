import React, { useMemo, useState, useCallback } from "react";
import { Pencil1Icon, PlusIcon } from "@radix-ui/react-icons";
import { Flex, Text, Box, Spinner, Button } from "@radix-ui/themes";
import { ToolCard, ToolStatus } from "./ToolCard";
import { useStoredOpen } from "../useStoredOpen";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import {
  selectManyDiffMessageByIds,
  selectIsStreaming,
  selectIsWaiting,
  selectToolResultById,
} from "../../../features/Chat/Thread/selectors";
import { selectChatId, selectCanPaste } from "../../../features/Chat";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
import { toolsApi } from "../../../services/refact";
import {
  parseRawTextDocToolCall,
  isRawTextDocToolCall,
  isCreateTextDocToolCall,
  isUpdateTextDocToolCall,
  isUpdateTextDocByLinesToolCall,
} from "../../Tools/types";
import { basename } from "./utils";
import styles from "./EditTool.module.css";

interface EditToolProps {
  toolCall: ToolCall;
  diffs?: DiffChunk[];
  isActiveTool?: boolean;
}

function countNonEmptyLines(text: string): number {
  let count = 0;
  let hasContent = false;

  for (const char of text) {
    if (char === "\n") {
      if (hasContent) count++;
      hasContent = false;
    } else if (char !== "\r" && char !== " " && char !== "\t") {
      hasContent = true;
    }
  }

  return hasContent ? count + 1 : count;
}

function getDiffStats(diffs: DiffChunk[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const diff of diffs) {
    added += countNonEmptyLines(diff.lines_add);
    removed += countNonEmptyLines(diff.lines_remove);
  }
  return { added, removed };
}

function getFilePath(toolCall: ToolCall): string | null {
  try {
    const args = JSON.parse(toolCall.function.arguments) as { path?: string };
    return args.path ?? null;
  } catch {
    return null;
  }
}

function isCreateTool(name: string | undefined): boolean {
  return name === "create_textdoc";
}

const DiffLine: React.FC<{
  lineNumber?: number;
  sign: string;
  line: string;
}> = ({ lineNumber, sign, line }) => {
  const isRemove = sign === "-";
  const isAdd = sign === "+";
  const rowClass = isRemove ? styles.remove : isAdd ? styles.add : "";
  return (
    <div className={`${styles.diffLine} ${rowClass}`}>
      <span className={styles.lineNumber}>{lineNumber ?? ""}</span>
      <span className={styles.sign}>{sign}</span>
      <span className={styles.lineContent}>{line}</span>
    </div>
  );
};

function splitNonEmptyLines(text: string): string[] {
  const lines: string[] = [];
  let start = 0;

  for (let i = 0; i <= text.length; i++) {
    if (i !== text.length && text[i] !== "\n") continue;

    if (i > start) {
      lines.push(text.slice(start, i));
    }
    start = i + 1;
  }

  return lines;
}

const DiffBlock: React.FC<{ diff: DiffChunk }> = ({ diff }) => {
  const removeLines = useMemo(
    () => splitNonEmptyLines(diff.lines_remove),
    [diff.lines_remove],
  );
  const addLines = useMemo(
    () => splitNonEmptyLines(diff.lines_add),
    [diff.lines_add],
  );

  return (
    <Box className={styles.diffBlock}>
      {removeLines.map((line, i) => (
        <DiffLine
          key={`remove-${i}`}
          lineNumber={diff.line1 + i}
          sign="-"
          line={line}
        />
      ))}
      {addLines.map((line, i) => (
        <DiffLine
          key={`add-${i}`}
          lineNumber={diff.line1 + i}
          sign="+"
          line={line}
        />
      ))}
    </Box>
  );
};

interface FileEditItemProps {
  fileName: string;
  diffs: DiffChunk[];
  onOpenFile: () => void;
}

const FileEditItem: React.FC<FileEditItemProps> = ({
  fileName,
  diffs,
  onOpenFile,
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const stats = useMemo(() => getDiffStats(diffs), [diffs]);

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  const handleOpenClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onOpenFile();
    },
    [onOpenFile],
  );

  return (
    <div className={styles.fileItem}>
      <Flex
        className={styles.fileHeader}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <Text size="1" className={styles.filename} onClick={handleOpenClick}>
          {basename(fileName)}
        </Text>
        <Text size="1" className={styles.stats}>
          {stats.added > 0 && (
            <span className={styles.added}>+{stats.added}</span>
          )}
          {stats.removed > 0 && (
            <span className={styles.removed}>−{stats.removed}</span>
          )}
        </Text>
      </Flex>

      {isOpen && (
        <Box className={styles.diffContent}>
          {diffs.map((diff, i) => (
            <DiffBlock key={i} diff={diff} />
          ))}
        </Box>
      )}
    </div>
  );
};

export const EditTool: React.FC<EditToolProps> = ({
  toolCall,
  diffs = [],
  isActiveTool = true,
}) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey);
  const { queryPathThenOpenFile, diffPasteBack, sendToolCallToIde } =
    useEventsBusForIDE();
  const [requestDryRun, dryRunResult] = toolsApi.useDryRunForEditToolMutation();
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
  const canPaste = useAppSelector(selectCanPaste);
  const chatId = useAppSelector(selectChatId);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );

  const diffIds = useMemo(
    () => (toolCall.id ? [toolCall.id] : []),
    [toolCall.id],
  );
  const selectDiffs = useMemo(
    () => selectManyDiffMessageByIds(diffIds),
    [diffIds],
  );
  const toolDiffs = useAppSelector(selectDiffs);

  const hasResult = maybeResult !== undefined;
  const hasDiffs = diffs.length > 0 || toolDiffs.length > 0;
  const isToolBusy = isActiveTool && !hasResult && (isStreaming || isWaiting);
  const shouldRenderDiffs = hasDiffs && !isToolBusy;

  const allDiffs = useMemo(() => {
    if (!shouldRenderDiffs) return [];

    const fromProps = diffs;
    const fromStore = toolDiffs.flatMap((d) => d.content);
    return fromProps.length > 0 ? fromProps : fromStore;
  }, [diffs, shouldRenderDiffs, toolDiffs]);

  const parsedToolCall = useMemo(() => {
    if (!isRawTextDocToolCall(toolCall)) return null;
    return parseRawTextDocToolCall(toolCall);
  }, [toolCall]);

  const replaceContent = useMemo(() => {
    if (!parsedToolCall) return null;
    if (isCreateTextDocToolCall(parsedToolCall)) {
      return parsedToolCall.function.arguments.content;
    }
    if (isUpdateTextDocToolCall(parsedToolCall)) {
      return parsedToolCall.function.arguments.replacement;
    }
    if (isUpdateTextDocByLinesToolCall(parsedToolCall)) {
      return parsedToolCall.function.arguments.content;
    }
    return null;
  }, [parsedToolCall]);

  const handleApplyDiff = useCallback(() => {
    if (!parsedToolCall) return;
    requestDryRun({
      toolName: parsedToolCall.function.name,
      toolArgs: parsedToolCall.function.arguments,
    })
      .then((results) => {
        if (results.data) {
          sendToolCallToIde(parsedToolCall, results.data, chatId);
        }
      })
      .catch(() => {
        /* ignore */
      });
  }, [chatId, parsedToolCall, requestDryRun, sendToolCallToIde]);

  const handleReplace = useCallback(() => {
    if (replaceContent) {
      diffPasteBack(replaceContent, chatId, toolCall.id);
    }
  }, [chatId, diffPasteBack, replaceContent, toolCall.id]);

  const filePath = useMemo(() => {
    const fromArgs = getFilePath(toolCall);
    if (fromArgs) return fromArgs;
    if (allDiffs.length > 0) return allDiffs[0].file_name;
    return null;
  }, [toolCall, allDiffs]);
  const isCreate = isCreateTool(toolCall.function.name);
  const stats = useMemo(() => getDiffStats(allDiffs), [allDiffs]);

  const filesByName = useMemo(() => {
    const grouped: Record<string, DiffChunk[]> = {};
    for (const diff of allDiffs) {
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
      grouped[diff.file_name] = (grouped[diff.file_name] || []).concat(diff);
    }
    return grouped;
  }, [allDiffs]);

  const fileNames = Object.keys(filesByName);
  const isSingleFile = fileNames.length <= 1;

  const handleFileClick = useCallback(
    (e: React.MouseEvent, path: string) => {
      e.stopPropagation();
      void queryPathThenOpenFile({ file_path: path });
    },
    [queryPathThenOpenFile],
  );

  const status: ToolStatus = useMemo(() => {
    if (
      maybeResult &&
      typeof maybeResult === "object" &&
      "tool_failed" in maybeResult &&
      maybeResult.tool_failed
    ) {
      return "error";
    }
    if (isToolBusy) return "running";
    // Has result but no diffs - could be an error message
    if (hasResult && !hasDiffs) {
      return "error";
    }
    return "success";
  }, [hasDiffs, hasResult, isToolBusy, maybeResult]);

  const summary = useMemo(() => {
    const statsEl =
      stats.added > 0 || stats.removed > 0 ? (
        <span className={styles.statsInline}>
          {stats.added > 0 && (
            <span className={styles.added}>+{stats.added}</span>
          )}
          {stats.removed > 0 && (
            <span className={styles.removed}>−{stats.removed}</span>
          )}
        </span>
      ) : null;

    const verb = isCreate ? "Create" : "Edit";
    if (isSingleFile && filePath) {
      return (
        <>
          {verb}{" "}
          <span
            className={styles.filename}
            onClick={(e) => handleFileClick(e, filePath)}
          >
            {basename(filePath)}
          </span>
          {statsEl && <> {statsEl}</>}
        </>
      );
    }
    if (fileNames.length > 1) {
      return (
        <>
          {verb} {fileNames.length} files {statsEl}
        </>
      );
    }
    return (
      <>
        {verb} file {statsEl}
      </>
    );
  }, [
    isCreate,
    isSingleFile,
    filePath,
    fileNames.length,
    handleFileClick,
    stats.added,
    stats.removed,
  ]);

  const icon = isCreate ? <PlusIcon /> : <Pencil1Icon />;

  return (
    <ToolCard
      icon={icon}
      summary={summary}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {status === "error" &&
        maybeResult?.content &&
        typeof maybeResult.content === "string" && (
          <Box className={styles.errorContent}>
            <Text size="1" color="red">
              {maybeResult.content}
            </Text>
          </Box>
        )}
      {shouldRenderDiffs && (
        <>
          <Flex gap="2" className={styles.actionBar}>
            <Button
              size="1"
              variant="soft"
              onClick={handleApplyDiff}
              disabled={dryRunResult.isLoading || !parsedToolCall}
            >
              {dryRunResult.isLoading ? (
                <Spinner size="1" />
              ) : (
                <Flex as="span" align="center" gap="1">
                  <PlusIcon />
                  Diff
                </Flex>
              )}
            </Button>
            {replaceContent && (
              <Button
                size="1"
                variant="soft"
                onClick={handleReplace}
                disabled={!canPaste}
              >
                <Flex as="span" align="center" gap="1">
                  <PlusIcon />
                  Replace
                </Flex>
              </Button>
            )}
          </Flex>
          {isSingleFile ? (
            <Box className={styles.diffContent}>
              {allDiffs.map((diff, i) => (
                <DiffBlock key={i} diff={diff} />
              ))}
            </Box>
          ) : (
            <Flex direction="column" gap="1" className={styles.fileList}>
              {fileNames.map((fileName) => (
                <FileEditItem
                  key={fileName}
                  fileName={fileName}
                  diffs={filesByName[fileName]}
                  onOpenFile={() =>
                    void queryPathThenOpenFile({ file_path: fileName })
                  }
                />
              ))}
            </Flex>
          )}
        </>
      )}
    </ToolCard>
  );
};

export default EditTool;
