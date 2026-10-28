import React, { useMemo } from "react";

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

function capText(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text;
  return `${text.slice(0, maxChars)}\n… output capped in UI (${
    text.length - maxChars
  } chars hidden)`;
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
  const totalChars = sections.reduce(
    (sum, section) => sum + section.content.length,
    0,
  );
  const renderedSections = useMemo(() => {
    let remaining = MAX_OUTPUT_CHARS;
    return sections.map((section) => {
      const rendered = capText(
        section.content,
        Math.max(0, Math.min(MAX_SECTION_CHARS, remaining)),
      );
      remaining = Math.max(0, remaining - rendered.length);
      return { ...section, rendered };
    });
  }, [sections]);
  const cursor = transcriptDetails(transcript);
  const isTruncated =
    transcript?.is_truncated === true || totalChars > MAX_OUTPUT_CHARS;
  const showTitles = renderedSections.length > 1;

  if (sections.length === 0 && !transcript) return null;

  return (
    <div className={styles.outputView} data-testid="exec-output-view">
      {isTruncated && (
        <p className={styles.note} data-testid="exec-truncation-notice">
          Output is truncated or capped. Use process_read with the process ID
          and cursor for more logs.
          {typeof transcript?.dropped_bytes === "number" &&
            transcript.dropped_bytes > 0 &&
            ` Dropped ${transcript.dropped_bytes} bytes from the runtime transcript.`}
        </p>
      )}
      {renderedSections.length === 0 ? (
        <p className={styles.note}>No output captured yet.</p>
      ) : (
        renderedSections.map((section) => (
          <div className={styles.outputSection} key={section.title}>
            {showTitles && (
              <span className={styles.streamLabel}>{section.title}</span>
            )}
            {section.rendered ? (
              <pre className={styles.outputPre}>{section.rendered}</pre>
            ) : (
              <span className={styles.note}>empty</span>
            )}
          </div>
        ))
      )}
      {cursor.length > 0 && (
        <p className={styles.note}>Cursor: {cursor.join(" · ")}</p>
      )}
    </div>
  );
};

export default ProcessOutputView;
