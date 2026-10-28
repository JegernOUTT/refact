import {
  CircleCheck,
  FilePen,
  FilePlus,
  FileX,
  LoaderCircle,
  RotateCcw,
} from "lucide-react";
import React, { useMemo, useCallback } from "react";
import { Box } from "@radix-ui/themes";
import { Icon } from "../../ui";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useEventsBusForIDE } from "../../../hooks/useEventBusForIDE";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
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
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptStatus,
} from "../TranscriptItem";
import { DiffBlock, type DiffHeaderAction } from "./DiffBlock";
import {
  diffStatsMeta,
  editToolFilePaths,
  fileListTarget,
} from "./DiffBlockStats";
import styles from "./EditTool.module.css";

interface EditToolProps {
  toolCall: ToolCall;
  diffs?: DiffChunk[];
  isActiveTool?: boolean;
}

type EditVerb = "Edit" | "Create" | "Delete";

const CREATE_ACTIONS = new Set(["new", "add", "create"]);
const DELETE_ACTIONS = new Set(["remove", "delete"]);

function editVerb(toolName: string | undefined, diffs: DiffChunk[]): EditVerb {
  if (diffs.length > 0) {
    if (diffs.every((diff) => CREATE_ACTIONS.has(diff.file_action))) {
      return "Create";
    }
    if (diffs.every((diff) => DELETE_ACTIONS.has(diff.file_action))) {
      return "Delete";
    }
    return "Edit";
  }
  return toolName === "create_textdoc" ? "Create" : "Edit";
}

const VERB_ICONS: Record<EditVerb, React.ReactNode> = {
  Edit: <FilePen />,
  Create: <FilePlus />,
  Delete: <FileX />,
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
  const result = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, chatId, toolCall.id),
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
  const isToolBusy =
    isActiveTool && result === undefined && (isStreaming || isWaiting);
  const shouldRenderDiffs = hasDiffs && !isToolBusy;
  const hasSelection = selectedSnippet.code.trim().length > 0;

  const allDiffs = useMemo(() => {
    if (!shouldRenderDiffs) return [];
    const fromStore = toolDiffs.flatMap((d) => d.content);
    return diffs.length > 0 ? diffs : fromStore;
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
      .catch(() => undefined);
  }, [chatId, parsedToolCall, requestDryRun, sendToolCallToIde]);

  const handleReplace = useCallback(() => {
    if (replaceContent !== null) {
      diffPasteBack(replaceContent, chatId, toolCall.id);
    }
  }, [chatId, diffPasteBack, replaceContent, toolCall.id]);

  const argsPaths = useMemo(
    () => editToolFilePaths(toolCall.function.arguments),
    [toolCall],
  );
  const argsPath = argsPaths.length === 1 ? argsPaths[0] : null;

  const fileNames = useMemo(() => {
    const names = Array.from(new Set(allDiffs.map((diff) => diff.file_name)));
    return names.length > 0 ? names : argsPaths;
  }, [allDiffs, argsPaths]);
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

  const verb = editVerb(toolCall.function.name, allDiffs);
  const failed = result?.tool_failed === true;
  const status: TranscriptStatus = isToolBusy
    ? "running"
    : failed
      ? "failed"
      : "done";
  const errorText =
    failed && typeof result.content === "string" ? result.content : null;

  if (!shouldRenderDiffs && !isToolBusy && !errorText) return null;

  const output =
    allDiffs.length > 0 ? (
      <TranscriptClamp>
        <Box className={`${styles.fileList} scrollX`}>
          {allDiffs.map((diff, i) => {
            const fileName =
              isSingleFile && argsPath ? argsPath : diff.file_name;
            return (
              <DiffBlock
                key={`${diff.file_name}-${i}`}
                diff={diff}
                fileName={fileName}
                onOpenFile={
                  canOpen ? () => openFile({ path: fileName }) : undefined
                }
                actions={diffActions}
                uncapped
              />
            );
          })}
        </Box>
      </TranscriptClamp>
    ) : null;

  return (
    <TranscriptItem
      data-testid="edit-tool"
      defaultOpen
      error={errorText}
      icon={VERB_ICONS[verb]}
      meta={diffStatsMeta(allDiffs)}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      status={status}
      target={fileListTarget(fileNames)}
      verb={verb}
    />
  );
};

export default EditTool;
