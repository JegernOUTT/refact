import { CircleCheck, LoaderCircle, RotateCcw } from "lucide-react";
import React, { useMemo, useCallback } from "react";
import { Flex, Box } from "@radix-ui/themes";
import { Icon } from "../../ui";
import {
  useAppSelector,
  useEventsBusForIDE,
  useOpenFileInApp,
} from "../../../hooks";
import { selectCapabilities } from "../../../features/Config/configSlice";
import {
  selectManyDiffMessageByThreadAndIds,
  selectIsStreamingById,
  selectIsWaitingById,
  selectToolResultByThreadAndId,
} from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { selectCanPaste, selectSelectedSnippet } from "../../../features/Chat";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
import { toolsApi } from "../../../services/refact";
import {
  parseRawTextDocToolCall,
  isRawTextDocToolCall,
  isCreateTextDocToolCall,
  isUpdateTextDocToolCall,
  isUpdateTextDocByLinesToolCall,
} from "../../Tools/types";

import { DiffBlock, type DiffHeaderAction } from "./DiffBlock";
import styles from "./EditTool.module.css";

interface EditToolProps {
  toolCall: ToolCall;
  diffs?: DiffChunk[];
  isActiveTool?: boolean;
}

function getFilePath(toolCall: ToolCall): string | null {
  try {
    const args = JSON.parse(toolCall.function.arguments) as Record<
      string,
      unknown
    >;
    return typeof args.path === "string" ? args.path : null;
  } catch {
    return null;
  }
}

interface FileEditItemProps {
  fileName: string;
  diffs: DiffChunk[];
  onOpenFile?: (fileName: string) => void;
  actions?: DiffHeaderAction[];
}

const FileEditItem: React.FC<FileEditItemProps> = ({
  fileName,
  diffs,
  onOpenFile,
  actions = [],
}) => {
  return (
    <div className={styles.fileItem}>
      <Box className="scrollX">
        {diffs.map((diff, i) => (
          <DiffBlock
            key={i}
            diff={diff}
            fileName={fileName}
            onOpenFile={onOpenFile ? () => onOpenFile(fileName) : undefined}
            actions={actions}
          />
        ))}
      </Box>
    </div>
  );
};

export const EditTool: React.FC<EditToolProps> = ({
  toolCall,
  diffs = [],
  isActiveTool = true,
}) => {
  const { diffPasteBack, sendToolCallToIde } = useEventsBusForIDE();
  const { canOpen, openFile } = useOpenFileInApp();
  const capabilities = useAppSelector(selectCapabilities);
  const [requestDryRun, dryRunResult] = toolsApi.useDryRunForEditToolMutation();
  const chatId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, chatId),
  );
  const canPaste = useAppSelector(selectCanPaste);
  const selectedSnippet = useAppSelector(selectSelectedSnippet);

  const hasResult = useAppSelector(
    (state) =>
      selectToolResultByThreadAndId(state, chatId, toolCall.id) !== undefined,
  );

  const diffIds = useMemo(
    () => (toolCall.id ? [toolCall.id] : []),
    [toolCall.id],
  );
  const selectDiffs = useMemo(
    () => selectManyDiffMessageByThreadAndIds(chatId, diffIds),
    [chatId, diffIds],
  );
  const toolDiffs = useAppSelector(selectDiffs);

  const hasDiffs = diffs.length > 0 || toolDiffs.length > 0;
  const isToolBusy = isActiveTool && !hasResult && (isStreaming || isWaiting);
  const shouldRenderDiffs = hasDiffs && !isToolBusy;
  const hasSelection = selectedSnippet.code.trim().length > 0;

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
    if (replaceContent !== null) {
      diffPasteBack(replaceContent, chatId, toolCall.id);
    }
  }, [chatId, diffPasteBack, replaceContent, toolCall.id]);

  const filePath = useMemo(() => {
    const fromArgs = getFilePath(toolCall);
    if (fromArgs) return fromArgs;
    if (allDiffs.length > 0) return allDiffs[0].file_name;
    return null;
  }, [toolCall, allDiffs]);

  const filesByName = useMemo(() => {
    const grouped: Partial<Record<string, DiffChunk[]>> = {};
    for (const diff of allDiffs) {
      const fileDiffs = grouped[diff.file_name] ?? [];
      grouped[diff.file_name] = fileDiffs.concat(diff);
    }
    return grouped;
  }, [allDiffs]);

  const fileNames = Object.keys(filesByName).filter(
    (fileName): fileName is string => filesByName[fileName] !== undefined,
  );
  const isSingleFile = fileNames.length <= 1;

  const diffActions = useMemo(() => {
    if (!capabilities.ideDiffPasteBack) return [];

    const actions: DiffHeaderAction[] = [
      {
        label: "Apply diff",
        icon: dryRunResult.isLoading ? (
          <Icon icon={LoaderCircle} size="sm" tone="accent" />
        ) : (
          <Icon icon={CircleCheck} size="sm" tone="success" />
        ),
        onClick: handleApplyDiff,
        disabled: dryRunResult.isLoading || !parsedToolCall,
      },
    ];

    if (replaceContent !== null && hasSelection) {
      actions.push({
        label: "Replace selection",
        icon: <Icon icon={RotateCcw} size="sm" />,
        onClick: handleReplace,
        disabled: !canPaste,
      });
    }

    return actions;
  }, [
    canPaste,
    capabilities.ideDiffPasteBack,
    dryRunResult.isLoading,
    handleApplyDiff,
    handleReplace,
    hasSelection,
    parsedToolCall,
    replaceContent,
  ]);

  if (!shouldRenderDiffs) return null;

  return isSingleFile ? (
    <Box className="scrollX">
      {allDiffs.map((diff, i) => (
        <DiffBlock
          key={i}
          diff={diff}
          fileName={filePath ?? diff.file_name}
          onOpenFile={
            canOpen
              ? () => openFile({ path: filePath ?? diff.file_name })
              : undefined
          }
          actions={diffActions}
        />
      ))}
    </Box>
  ) : (
    <Flex direction="column" gap="1" className={styles.fileList}>
      {fileNames.map((fileName) => (
        <FileEditItem
          key={fileName}
          fileName={fileName}
          diffs={filesByName[fileName] ?? []}
          onOpenFile={canOpen ? (path) => openFile({ path }) : undefined}
          actions={diffActions}
        />
      ))}
    </Flex>
  );
};

export default EditTool;
