import { CirclePlus, ExternalLink, MoveRight, Trash2 } from "lucide-react";
import React, { useMemo } from "react";
import { Box } from "@radix-ui/themes";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import {
  selectToolResultByThreadAndId,
  selectManyDiffMessageByThreadAndIds,
} from "../../../features/Chat/Thread/selectors";
import { useThreadId } from "../../../features/Chat/Thread";
import { ToolCall, DiffChunk } from "../../../services/refact/types";
import { ShikiCodeBlock } from "../../Markdown";
import { Button } from "../../ui";
import {
  TranscriptClamp,
  TranscriptItem,
  type TranscriptFact,
  type TranscriptStatus,
} from "../TranscriptItem";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import { basename } from "./utils";
import { DiffBlock } from "./DiffBlock";
import { getDiffStats } from "./DiffBlockStats";

type FileOpType = "mv" | "rm" | "add_workspace_folder";

interface FileOpArgs {
  source?: string;
  destination?: string;
  path?: string;
  recursive?: boolean;
}

interface FileOpToolProps {
  toolCall: ToolCall;
  toolType: FileOpType;
  diffs?: DiffChunk[];
  isActiveTool?: boolean;
}

function parseArgs(raw: string): FileOpArgs {
  try {
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object" ? (parsed as FileOpArgs) : {};
  } catch {
    return {};
  }
}

export const FileOpTool: React.FC<FileOpToolProps> = ({
  toolCall,
  toolType,
  diffs = [],
  isActiveTool = true,
}) => {
  const { canOpen, openFile } = useOpenFileInApp();
  const threadId = useThreadId();
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const maybeResult = useAppSelector((state) =>
    selectToolResultByThreadAndId(state, threadId, toolCall.id),
  );
  const diffIds = useMemo(
    () => (toolCall.id ? [toolCall.id] : []),
    [toolCall.id],
  );
  const selectDiffs = useMemo(
    () => selectManyDiffMessageByThreadAndIds(threadId, diffIds),
    [threadId, diffIds],
  );
  const toolDiffs = useAppSelector(selectDiffs);

  const hasResult = maybeResult !== undefined;
  const hasDiffs = diffs.length > 0 || toolDiffs.length > 0;
  const isToolBusy =
    isActiveTool && !hasResult && unrecorded.status === "running";

  const allDiffs = useMemo((): DiffChunk[] => {
    if (!hasDiffs || isToolBusy) return [];
    return diffs.length > 0 ? diffs : toolDiffs.flatMap((d) => d.content);
  }, [diffs, hasDiffs, isToolBusy, toolDiffs]);

  const args = useMemo(
    () => parseArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );

  const failed = maybeResult?.tool_failed === true;
  const unrecordedRow = !hasResult && !(hasDiffs && !isToolBusy);
  const status: TranscriptStatus = failed
    ? "failed"
    : unrecordedRow
      ? unrecorded.status
      : "done";

  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;

  const path = args.path ?? "";
  const source = args.source ?? "";
  const destination = args.destination ?? "";
  const removed = getDiffStats(allDiffs).removed;

  const row =
    toolType === "mv"
      ? {
          icon: <MoveRight />,
          verb: "Move",
          target: `${basename(source)} → ${basename(destination)}`,
          meta: undefined,
        }
      : toolType === "add_workspace_folder"
        ? {
            icon: <CirclePlus />,
            verb: "Add workspace",
            target: basename(path),
            meta: undefined,
          }
        : {
            icon: <Trash2 />,
            verb: "Delete",
            target: basename(path),
            meta:
              removed > 0
                ? `−${removed}`
                : args.recursive
                  ? "recursive"
                  : undefined,
          };

  const summary: TranscriptFact[] =
    toolType === "mv"
      ? [
          { label: "From", value: source, title: source },
          { label: "To", value: destination, title: destination },
        ]
      : path
        ? [{ label: "Path", value: path, title: path }]
        : [];

  const openPaths = toolType === "mv" ? [destination].filter(Boolean) : [];

  const output =
    allDiffs.length > 0 ? (
      <TranscriptClamp>
        <Box className="scrollX">
          {allDiffs.map((diff, i) => (
            <DiffBlock
              key={`${diff.file_name}-${i}`}
              diff={diff}
              fileName={diff.file_name}
              uncapped
            />
          ))}
        </Box>
      </TranscriptClamp>
    ) : content && !failed ? (
      <TranscriptClamp>
        <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
      </TranscriptClamp>
    ) : null;

  const actions =
    canOpen && openPaths.length > 0
      ? openPaths.map((filePath) => (
          <Button
            key={filePath}
            leftIcon={ExternalLink}
            size="sm"
            type="button"
            variant="soft"
            onClick={() => openFile({ path: filePath })}
          >
            Open {basename(filePath)}
          </Button>
        ))
      : null;

  return (
    <TranscriptItem
      data-testid="file-op-tool"
      defaultOpen={allDiffs.length > 0}
      actions={actions}
      error={failed ? content : null}
      icon={row.icon}
      meta={row.meta}
      openKey={toolCall.id ? `tc:${toolCall.id}` : undefined}
      output={output}
      raw={[{ label: "Arguments", value: toolCall.function.arguments }]}
      status={status}
      statusLabel={
        unrecordedRow && !failed ? unrecorded.statusLabel : undefined
      }
      summary={summary}
      target={row.target}
      verb={row.verb}
    />
  );
};

export default FileOpTool;
