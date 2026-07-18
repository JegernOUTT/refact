import React, { useCallback, useMemo, useRef } from "react";
import classNames from "classnames";
import { Text, Container, Box, Flex, Link } from "@radix-ui/themes";
import { DiffMessage, type DiffChunk } from "../../services/refact";
import styles from "./ChatContent.module.css";
import { filename } from "../../utils";
import * as Collapsible from "@radix-ui/react-collapsible";
import { Chevron } from "../Collapsible";
import groupBy from "lodash.groupby";
import { TruncateLeft } from "../Text";
import { useHideScroll, useOpenFileInApp } from "../../hooks";
import { FadedButton } from "../Buttons";

type DiffType = "apply" | "unapply" | "error" | "can not apply";

const DiffLine: React.FC<{
  lineNumber?: number;
  sign: string;
  line: string;
}> = ({ lineNumber, sign, line }) => {
  const toneClass =
    sign === "-" ? styles.diff_line_remove : styles.diff_line_add;
  return (
    <Flex className={classNames(styles.diff_line, toneClass)}>
      <Text size="2" className={styles.diff_line_number}>
        {lineNumber ?? ""}
      </Text>
      <Text size="2" className={styles.diff_sign}>
        {sign}
      </Text>
      <Text size="2" className={styles.diff_line_content}>
        {line}
      </Text>
    </Flex>
  );
};

function splitDiffLines(text: string): string[] {
  const lines: string[] = [];
  let start = 0;

  for (let i = 0; i <= text.length; i++) {
    if (i !== text.length && text[i] !== "\n") continue;
    lines.push(text.slice(start, i));
    start = i + 1;
  }

  return lines;
}

const DiffHighlight: React.FC<{
  startLine?: number;
  sign: string;
  text: string;
}> = ({ startLine, sign, text }) => {
  const lines = useMemo(() => splitDiffLines(text), [text]);
  return (
    <Flex direction="column" className={styles.diff_highlight}>
      {lines.map((line, index) => {
        return (
          <DiffLine
            key={index}
            line={line}
            sign={sign}
            lineNumber={startLine ? index + startLine : undefined}
          />
        );
      })}
    </Flex>
  );
};

type DiffProps = {
  diff: DiffChunk;
};

export const Diff: React.FC<DiffProps> = ({ diff }) => {
  const removeString = diff.lines_remove && diff.lines_remove;
  const addString = diff.lines_add && diff.lines_add;
  const isRename = diff.file_action === "rename" && diff.file_name_rename;

  return (
    <Flex className={styles.diff} py="2" direction="column">
      {isRename && (
        <Flex py="1" px="2">
          <Text size="1" className={styles.diffRenameText}>
            {filename(diff.file_name)} was renamed to{" "}
            {filename(diff.file_name_rename ?? "")}
          </Text>
        </Flex>
      )}
      {removeString && !isRename && (
        <DiffHighlight startLine={diff.line1} sign={"-"} text={removeString} />
      )}
      {addString && !isRename && (
        <DiffHighlight startLine={diff.line1} sign={"+"} text={addString} />
      )}
    </Flex>
  );
};

export type DiffChunkWithTypeAndApply = DiffChunk & {
  type: DiffType;
  apply: boolean;
};

function countDiffLines(text: string): number {
  if (!text) return 0;

  let count = 1;
  for (const char of text) {
    if (char === "\n") count++;
  }
  return count;
}

function buildDiffTitleNodes(
  diffs: Record<string, DiffChunk[]>,
): React.ReactNode[] {
  const entries = Object.entries(diffs);
  const nodes: React.ReactNode[] = [];

  for (const [fullPath, diffForFile] of entries) {
    const name = filename(fullPath);

    const renameAction = diffForFile.find(
      (diff) => diff.file_action === "rename" && diff.file_name_rename,
    );

    let addCount = 0;
    let removeCount = 0;
    for (const diff of diffForFile) {
      addCount += countDiffLines(diff.lines_add);
      removeCount += countDiffLines(diff.lines_remove);
    }

    if (nodes.length > 0) {
      nodes.push(", ");
    }

    if (renameAction?.file_name_rename) {
      const newName = filename(renameAction.file_name_rename);
      nodes.push(
        <Text
          className={styles.diffTitleNode}
          key={fullPath + "-" + diffForFile.length}
        >
          {name} <span className={styles.diffRenameText}>→ {newName}</span>
        </Text>,
      );
    } else {
      nodes.push(
        <Text
          className={styles.diffTitleNode}
          key={fullPath + "-" + diffForFile.length}
        >
          {name}{" "}
          {addCount > 0 && (
            <span className={styles.diffAddedText}>+{addCount}</span>
          )}
          {addCount > 0 && removeCount > 0 && " "}
          {removeCount > 0 && (
            <span className={styles.diffRemovedText}>-{removeCount}</span>
          )}
        </Text>,
      );
    }
  }

  return nodes;
}

export const DiffTitle: React.FC<{
  diffs: Record<string, DiffChunk[]>;
}> = ({ diffs }) => {
  const nodes = useMemo(() => buildDiffTitleNodes(diffs), [diffs]);
  return <>{nodes}</>;
};

export const DiffContent: React.FC<{
  diffs: Record<string, DiffChunk[]>;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}> = ({ diffs, open: controlledOpen, onOpenChange }) => {
  const [internalOpen, setInternalOpen] = React.useState(false);
  const contentId = React.useId();
  const ref = useRef<HTMLButtonElement>(null);
  const handleScroll = useHideScroll(ref);

  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;

  const setOpen = useCallback(
    (value: boolean) => {
      if (isControlled && onOpenChange) {
        onOpenChange(value);
      } else {
        setInternalOpen(value);
      }
    },
    [isControlled, onOpenChange],
  );

  const handleHide = useCallback(() => {
    setOpen(false);
    handleScroll();
  }, [handleScroll, setOpen]);

  return (
    <Collapsible.Root open={open} onOpenChange={(v) => setOpen(v)}>
      <Collapsible.Trigger asChild>
        <button
          type="button"
          ref={ref}
          className={styles.diffHeader}
          aria-controls={contentId}
        >
          <Text weight="light" size="1">
            <DiffTitle diffs={diffs} />
          </Text>
          <Chevron open={open} className={styles.diffChevron} />
        </button>
      </Collapsible.Trigger>
      <Collapsible.Content forceMount asChild>
        <div
          id={contentId}
          className={classNames("rf-expand-grid", styles.diffContentGrid)}
          data-open={open}
          hidden={false}
        >
          <div className={styles.diffContentBody}>
            <Flex direction="column">
              <DiffForm diffs={diffs} />
              <FadedButton color="gray" onClick={handleHide} mx="2">
                Hide Diff
              </FadedButton>
            </Flex>
          </div>
        </div>
      </Collapsible.Content>
    </Collapsible.Root>
  );
};

export type DiffWithStatus = DiffChunk & {
  state?: 0 | 1 | 2;
  can_apply: boolean;
  applied: boolean;
  index: number;
};

export const DiffForm: React.FC<{
  diffs: Record<string, DiffChunk[]>;
}> = ({ diffs }) => {
  const { canOpen, openFile } = useOpenFileInApp();
  return (
    <Flex direction="column" maxWidth="100%" py="2" gap="2">
      {Object.entries(diffs).map(([fullFilePath, diffsForFile], index) => {
        const key = fullFilePath + "-" + index;

        // Check if this is a rename action
        const renameAction = diffsForFile.find(
          (diff) => diff.file_action === "rename" && diff.file_name_rename,
        );

        return (
          <Box key={key} my="2">
            <Flex justify="between" align="center" p="1">
              <TruncateLeft size="1">
                {canOpen ? (
                  <Link
                    // TODO: check how ides treat this being "", undefined, or "#"
                    href=""
                    onClick={(event) => {
                      event.preventDefault();
                      const startLine = Math.min(
                        ...diffsForFile.map((diff) => diff.line1),
                      );
                      openFile({
                        path: fullFilePath,
                        line: startLine,
                        resolved: true,
                      });
                    }}
                  >
                    <Text
                      as="span"
                      className={classNames({
                        [styles.diffRenameText]: Boolean(
                          renameAction?.file_name_rename,
                        ),
                        [styles.diffFilePath]: !renameAction?.file_name_rename,
                      })}
                    >
                      {renameAction?.file_name_rename
                        ? renameAction.file_name_rename
                        : fullFilePath}
                    </Text>
                  </Link>
                ) : (
                  <Text
                    as="span"
                    className={classNames({
                      [styles.diffRenameText]: Boolean(
                        renameAction?.file_name_rename,
                      ),
                      [styles.diffFilePath]: !renameAction?.file_name_rename,
                    })}
                  >
                    {renameAction?.file_name_rename
                      ? renameAction.file_name_rename
                      : fullFilePath}
                  </Text>
                )}
              </TruncateLeft>
            </Flex>
            <Box className="scrollX">
              <Box className={styles.diff_scroll_inner}>
                <Box className={styles.diff_file_body}>
                  {diffsForFile.map((diff, i) => (
                    <Diff key={`${fullFilePath}-${index}-${i}`} diff={diff} />
                  ))}
                </Box>
              </Box>
            </Box>
          </Box>
        );
      })}
    </Flex>
  );
};

type GroupedDiffsProps = {
  diffs: DiffMessage[];
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
};

const _GroupedDiffs: React.FC<GroupedDiffsProps> = ({
  diffs,
  open,
  onOpenChange,
}) => {
  const groupedByFileName = useMemo(() => {
    const chunks: DiffMessage["content"] = [];
    for (const diff of diffs) {
      chunks.push(...diff.content);
    }

    return groupBy(chunks, (chunk) => chunk.file_name);
  }, [diffs]);

  return (
    <Container>
      <Flex direction="column" gap="4" py="4" className="rf-enter-rise">
        <DiffContent
          diffs={groupedByFileName}
          open={open}
          onOpenChange={onOpenChange}
        />
      </Flex>
    </Container>
  );
};

export const GroupedDiffs = React.memo(_GroupedDiffs);
