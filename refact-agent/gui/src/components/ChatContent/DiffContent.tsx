import React, { useMemo } from "react";
import groupBy from "lodash.groupby";
import { FileDiff } from "lucide-react";
import { DiffMessage, type DiffChunk } from "../../services/refact";
import { filename } from "../../utils";
import { useOpenFileInApp } from "../../hooks/useOpenFileInApp";
import { Icon } from "../ui";
import { DiffBlock } from "./ToolCard/DiffBlock";
import { TranscriptClamp, TranscriptItem } from "./TranscriptItem";
import styles from "./DiffContent.module.css";

const CLAMP_LINE_THRESHOLD = 40;
const MAX_TARGET_FILES = 3;

function countDiffLines(text: string): number {
  if (!text) return 0;
  let count = 1;
  for (const char of text) {
    if (char === "\n") count++;
  }
  return text.endsWith("\n") ? count - 1 : count;
}

function diffStats(diffs: Record<string, DiffChunk[]>) {
  let addCount = 0;
  let removeCount = 0;
  for (const chunks of Object.values(diffs)) {
    for (const chunk of chunks) {
      addCount += countDiffLines(chunk.lines_add);
      removeCount += countDiffLines(chunk.lines_remove);
    }
  }
  return { addCount, removeCount };
}

function diffTarget(diffs: Record<string, DiffChunk[]>): string {
  const names = Object.entries(diffs).map(([path, chunks]) => {
    const rename = chunks.find(
      (chunk) => chunk.file_action === "rename" && chunk.file_name_rename,
    )?.file_name_rename;
    return rename ? `${filename(path)} → ${filename(rename)}` : filename(path);
  });
  if (names.length <= MAX_TARGET_FILES) return names.join(", ");
  const rest = names.length - MAX_TARGET_FILES;
  return `${names.slice(0, MAX_TARGET_FILES).join(", ")} +${rest}`;
}

function diffMeta(addCount: number, removeCount: number): string | null {
  const parts = [
    addCount > 0 ? `+${addCount}` : null,
    removeCount > 0 ? `−${removeCount}` : null,
  ].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" ") : null;
}

const DiffBlocks: React.FC<{ diffs: Record<string, DiffChunk[]> }> = ({
  diffs,
}) => {
  const { canOpen, openFile } = useOpenFileInApp();
  return (
    <div className={styles.blocks}>
      {Object.entries(diffs).flatMap(([path, chunks]) =>
        chunks.map((chunk, index) => (
          <DiffBlock
            key={`${path}-${index}`}
            diff={chunk}
            fileName={path}
            displayFileName={chunk.file_name_rename ?? path}
            onOpenFile={
              canOpen
                ? () => openFile({ path, line: chunk.line1, resolved: true })
                : undefined
            }
          />
        )),
      )}
    </div>
  );
};

export const DiffContent: React.FC<{
  diffs: Record<string, DiffChunk[]>;
  openKey?: string;
  onOpenChange?: (open: boolean) => void;
}> = ({ diffs, openKey, onOpenChange }) => {
  const { addCount, removeCount } = useMemo(() => diffStats(diffs), [diffs]);
  const blocks = <DiffBlocks diffs={diffs} />;
  const long = addCount + removeCount > CLAMP_LINE_THRESHOLD;

  return (
    <TranscriptItem
      data-testid="diff-content"
      icon={<Icon icon={FileDiff} size="sm" />}
      verb="Changes"
      target={diffTarget(diffs)}
      meta={diffMeta(addCount, removeCount)}
      defaultOpen
      openKey={openKey}
      onOpenChange={onOpenChange}
      sectionLabels={{ output: "" }}
      output={
        long ? (
          <TranscriptClamp lineCount={addCount + removeCount}>
            {blocks}
          </TranscriptClamp>
        ) : (
          blocks
        )
      }
    />
  );
};

type GroupedDiffsProps = {
  diffs: DiffMessage[];
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
};

const _GroupedDiffs: React.FC<GroupedDiffsProps> = ({
  diffs,
  onOpenChange,
}) => {
  const groupedByFileName = useMemo(() => {
    const chunks: DiffMessage["content"] = [];
    for (const diff of diffs) chunks.push(...diff.content);
    return groupBy(chunks, (chunk) => chunk.file_name);
  }, [diffs]);
  const openKey = diffs[0]?.tool_call_id
    ? `diff:${diffs[0].tool_call_id}`
    : undefined;

  return (
    <DiffContent
      diffs={groupedByFileName}
      openKey={openKey}
      onOpenChange={onOpenChange}
    />
  );
};

export const GroupedDiffs = React.memo(_GroupedDiffs);
