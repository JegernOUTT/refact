import React, { useMemo } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";

import type { ExecTranscriptMetadata } from "../../../services/refact/types";
import styles from "./ExecToolCard.module.css";

const MAX_OUTPUT_CHARS = 30_000;
const MAX_SECTION_CHARS = 16_000;

type ProcessOutputSection = {
  title: string;
  content: string;
};

type ProcessOutputViewProps = {
  content: string | null;
  transcript?: ExecTranscriptMetadata;
};

function stripCodeFence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed.startsWith("```")) return text.trimEnd();
  return trimmed
    .replace(/^```[a-zA-Z0-9_-]*\n?/, "")
    .replace(/```$/, "")
    .trimEnd();
}

function extractLegacySection(content: string, title: string): string | null {
  const fence = "```";
  const pattern = new RegExp(
    `${title}\\s*\\n${fence}\\n?([\\s\\S]*?)(?:${fence}|$)`,
    "i",
  );
  const match = pattern.exec(content);
  return match ? match[1].trimEnd() : null;
}

function extractProcessSection(content: string, title: string): string | null {
  const pattern = new RegExp(
    `(?:^|\\n)${title}:\\n([\\s\\S]*?)(?=\\n(?:stdout|stderr|combined|transcript):|$)`,
    "i",
  );
  const match = pattern.exec(content);
  if (!match) return null;
  const value = match[1].trimEnd();
  return value === "<empty>" ? "" : stripCodeFence(value);
}

function stripMetadataLines(content: string): string {
  const lines = content.split("\n");
  const firstOutputIndex = lines.findIndex((line) => {
    const normalized = line.trim().toLowerCase();
    return (
      normalized === "stdout:" ||
      normalized === "stderr:" ||
      normalized === "combined:" ||
      normalized === "stdout" ||
      normalized === "stderr"
    );
  });
  if (firstOutputIndex >= 0) return lines.slice(firstOutputIndex).join("\n");
  return content;
}

function parseSections(content: string | null): ProcessOutputSection[] {
  if (!content) return [];

  const stdout =
    extractProcessSection(content, "stdout") ??
    extractLegacySection(content, "STDOUT");
  const stderr =
    extractProcessSection(content, "stderr") ??
    extractLegacySection(content, "STDERR");
  const combined = extractProcessSection(content, "combined");
  const sections: ProcessOutputSection[] = [];

  if (stdout !== null) sections.push({ title: "stdout", content: stdout });
  if (stderr !== null) sections.push({ title: "stderr", content: stderr });
  if (combined !== null)
    sections.push({ title: "combined", content: combined });

  if (sections.length > 0) return sections;

  const fallback = stripMetadataLines(content).trimEnd();
  return fallback
    ? [{ title: "output", content: stripCodeFence(fallback) }]
    : [];
}

function capText(
  text: string,
  maxChars: number,
): { text: string; hiddenChars: number } {
  if (text.length <= maxChars) return { text, hiddenChars: 0 };
  return {
    text: `${text.slice(0, maxChars)}\n… output capped in UI (${
      text.length - maxChars
    } chars hidden)`,
    hiddenChars: text.length - maxChars,
  };
}

function transcriptDetails(
  transcript: ExecTranscriptMetadata | undefined,
): string[] {
  if (!transcript) return [];

  const details: string[] = [];
  if (typeof transcript.since_seq === "number") {
    details.push(`since ${transcript.since_seq}`);
  }
  if (typeof transcript.next_seq === "number") {
    details.push(`next ${transcript.next_seq}`);
  }
  if (typeof transcript.latest_seq === "number") {
    details.push(`latest ${transcript.latest_seq}`);
  }
  if (typeof transcript.current_bytes === "number") {
    details.push(`${transcript.current_bytes} bytes kept`);
  }
  return details;
}

export const ProcessOutputView: React.FC<ProcessOutputViewProps> = ({
  content,
  transcript,
}) => {
  const sections = useMemo(() => parseSections(content), [content]);
  const totalChars = useMemo(
    () => sections.reduce((sum, section) => sum + section.content.length, 0),
    [sections],
  );
  const renderedSections = useMemo(() => {
    let remaining = MAX_OUTPUT_CHARS;
    return sections.map((section) => {
      const sectionCap = Math.max(0, Math.min(MAX_SECTION_CHARS, remaining));
      const capped = capText(section.content, sectionCap);
      remaining = Math.max(0, remaining - capped.text.length);
      return {
        ...section,
        rendered: capped.text,
        hiddenChars: capped.hiddenChars,
      };
    });
  }, [sections]);
  const metadata = transcriptDetails(transcript);
  const cappedByTotal = totalChars > MAX_OUTPUT_CHARS;
  const isTruncated = transcript?.is_truncated === true || cappedByTotal;

  if (sections.length === 0 && !transcript) return null;

  return (
    <Box className={styles.outputView} data-testid="exec-output-view">
      {metadata.length > 0 && (
        <Text size="1" color="gray">
          Cursor: {metadata.join(" · ")}
        </Text>
      )}

      {isTruncated && (
        <Box
          className={styles.truncationNotice}
          data-testid="exec-truncation-notice"
        >
          <Text size="1">
            Output is truncated or capped. Use process_read with the process ID
            and cursor for more logs.
          </Text>
          {typeof transcript?.dropped_bytes === "number" &&
            transcript.dropped_bytes > 0 && (
              <Text size="1" color="gray" as="div">
                Dropped {transcript.dropped_bytes} bytes from the runtime
                transcript.
              </Text>
            )}
        </Box>
      )}

      {renderedSections.length === 0 ? (
        <Box className={styles.outputEmpty}>
          <Text size="1">No output captured yet.</Text>
        </Box>
      ) : (
        renderedSections.map((section) => (
          <Box key={section.title} className={styles.outputSection}>
            <Flex
              className={styles.outputHeader}
              justify="between"
              align="center"
              gap="2"
            >
              <Text size="1" weight="medium">
                {section.title}
              </Text>
              <Text size="1" color="gray">
                {section.content.length} chars
              </Text>
            </Flex>
            <Box className={styles.outputBody}>
              {section.rendered ? (
                <Box className="scrollX">
                  <pre className={styles.outputPre}>{section.rendered}</pre>
                </Box>
              ) : (
                <Text size="1" color="gray">
                  empty
                </Text>
              )}
            </Box>
          </Box>
        ))
      )}
    </Box>
  );
};

export default ProcessOutputView;
