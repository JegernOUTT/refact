import React, { useMemo, useState } from "react";
import classNames from "classnames";
import { Box, Button, Callout, Flex, Text } from "@radix-ui/themes";
import { FileTextIcon, GitHubLogoIcon } from "@radix-ui/react-icons";
import groupBy from "lodash.groupby";
import { useAppSelector } from "../../hooks";
import {
  selectIsStreaming,
  selectIsWaiting,
  selectToolResultById,
} from "../../features/Chat/Thread/selectors";
import type { DiffChunk, ToolCall } from "../../services/refact/types";
import { ShikiCodeBlock } from "../Markdown";
import { DiffForm } from "./DiffContent";
import { ToolCard, type ToolStatus } from "./ToolCard";
import { useStoredOpen } from "./useStoredOpen";
import { parseAgentDiffOutput, type AgentDiffReport } from "./AgentDiffModel";
import styles from "./AgentDiffView.module.css";

type AgentDiffContentProps = {
  report: AgentDiffReport;
};

type AgentDiffViewProps = {
  toolCall: ToolCall;
};

function countLines(text: string): number {
  if (!text) return 0;
  return text.split("\n").filter((line) => line.trim()).length;
}

function groupDiffChunks(chunks: DiffChunk[]): Record<string, DiffChunk[]> {
  return groupBy(chunks, (chunk) => chunk.file_name);
}

export const AgentDiffContent: React.FC<AgentDiffContentProps> = ({
  report,
}) => {
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const groupedDiffs = useMemo(
    () => groupDiffChunks(report.diffChunks),
    [report.diffChunks],
  );
  const selectedDiffs = useMemo(() => {
    if (!selectedFile) return groupedDiffs;
    return { [selectedFile]: groupedDiffs[selectedFile] };
  }, [groupedDiffs, selectedFile]);

  const showDiffForm = report.diffChunks.length > 0;
  const shouldShowFileTree = report.files.length > 1;

  return (
    <Box className={styles.root}>
      <Box className={styles.summary}>
        <Flex justify="between" align="center" gap="2" wrap="wrap">
          <Text weight="medium">Agent diff: {report.cardId}</Text>
          <Text size="1" color="gray">
            {report.mode}
          </Text>
        </Flex>
        <Box className={styles.metaGrid}>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Card</span>
            <span className={styles.value}>{report.cardTitle}</span>
          </Box>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Branch</span>
            <span className={styles.value}>{report.branch}</span>
          </Box>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Base</span>
            <span className={styles.value}>{report.base}</span>
          </Box>
          <Box className={styles.metaItem}>
            <span className={styles.label}>Files</span>
            <span className={styles.value}>{report.stats.files}</span>
          </Box>
        </Box>
        <Flex gap="3" className={styles.stats} wrap="wrap">
          <Text size="1" color="gray">
            {report.stats.files} files
          </Text>
          {report.stats.added > 0 && (
            <Text size="1" className={styles.added}>
              +{report.stats.added}
            </Text>
          )}
          {report.stats.removed > 0 && (
            <Text size="1" className={styles.removed}>
              −{report.stats.removed}
            </Text>
          )}
          <Text size="1" color="gray">
            {countLines(report.body)} lines
          </Text>
        </Flex>
        {report.truncated && (
          <Callout.Root color="amber" size="1" className={styles.truncation}>
            <Callout.Text>{report.truncated}</Callout.Text>
          </Callout.Root>
        )}
      </Box>

      {report.body.trim() === "(no changes detected)" ? (
        <Text as="div" size="2" className={styles.empty}>
          No changes detected.
        </Text>
      ) : (
        <Box className={styles.content}>
          {shouldShowFileTree && (
            <Box className={styles.fileTree}>
              <Text size="1" color="gray">
                Files
              </Text>
              <Flex direction="column" gap="1" className={styles.fileList}>
                <Button
                  size="1"
                  variant={selectedFile === null ? "solid" : "soft"}
                  className={styles.fileButton}
                  onClick={() => setSelectedFile(null)}
                >
                  <span className={styles.fileButtonInner}>All files</span>
                </Button>
                {report.files.map((file) => (
                  <Button
                    key={file}
                    size="1"
                    variant={selectedFile === file ? "solid" : "ghost"}
                    className={styles.fileButton}
                    onClick={() => setSelectedFile(file)}
                  >
                    <Flex
                      as="span"
                      align="center"
                      gap="1"
                      className={styles.fileButtonInner}
                    >
                      <FileTextIcon />
                      {file}
                    </Flex>
                  </Button>
                ))}
              </Flex>
            </Box>
          )}
          <Box
            className={classNames(
              styles.diffPane,
              !shouldShowFileTree && styles.diffPaneFull,
            )}
          >
            {showDiffForm ? (
              <DiffForm diffs={selectedDiffs} />
            ) : (
              <Box className={styles.rawDiff}>
                <ShikiCodeBlock
                  showLineNumbers={false}
                  className="language-text"
                >
                  {report.body}
                </ShikiCodeBlock>
              </Box>
            )}
          </Box>
        </Box>
      )}
    </Box>
  );
};

export const AgentDiffView: React.FC<AgentDiffViewProps> = ({ toolCall }) => {
  const storeKey = toolCall.id ? `tc:${toolCall.id}` : undefined;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);

  const maybeResult = useAppSelector((state) =>
    selectToolResultById(state, toolCall.id),
  );
  const content =
    maybeResult && typeof maybeResult.content === "string"
      ? maybeResult.content
      : null;
  const report = useMemo(
    () => (content ? parseAgentDiffOutput(content) : null),
    [content],
  );

  const status: ToolStatus = useMemo(() => {
    if (!maybeResult && (isStreaming || isWaiting)) return "running";
    if (!maybeResult) return "running";
    return maybeResult.tool_failed ? "error" : "success";
  }, [isStreaming, isWaiting, maybeResult]);

  const meta = report
    ? `${report.stats.files} files${
        report.stats.added || report.stats.removed
          ? ` +${report.stats.added} −${report.stats.removed}`
          : ""
      }`
    : undefined;

  return (
    <ToolCard
      icon={<GitHubLogoIcon />}
      summary={report ? `Agent diff: ${report.cardId}` : "Agent diff"}
      meta={meta}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
      toolCall={toolCall}
    >
      {report ? (
        <AgentDiffContent report={report} />
      ) : content ? (
        <ShikiCodeBlock showLineNumbers={false}>{content}</ShikiCodeBlock>
      ) : null}
    </ToolCard>
  );
};

export default AgentDiffView;
