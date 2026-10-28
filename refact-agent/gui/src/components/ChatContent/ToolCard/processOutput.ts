import type { ExecTranscriptMetadata } from "../../../services/refact/types";

export type ProcessOutputSection = {
  title: string;
  content: string;
};

const STREAMS = ["stdout", "stderr", "combined"] as const;

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

export function streamSections(content: string | null): ProcessOutputSection[] {
  if (!content) return [];
  return STREAMS.flatMap((stream) => {
    const legacy =
      stream === "combined"
        ? null
        : extractLegacySection(content, stream.toUpperCase());
    const value = extractProcessSection(content, stream) ?? legacy;
    return value === null ? [] : [{ title: stream, content: value }];
  });
}

export function parseProcessOutput(
  content: string | null,
): ProcessOutputSection[] {
  if (!content) return [];
  const sections = streamSections(content);
  if (sections.length > 0) return sections;
  const fallback = stripMetadataLines(content).trimEnd();
  return fallback
    ? [{ title: "output", content: stripCodeFence(fallback) }]
    : [];
}

export function processOutputLineCount(content: string | null): number {
  return parseProcessOutput(content).reduce(
    (sum, section) => sum + section.content.split("\n").length,
    0,
  );
}

export function lastStreamLine(content: string | null): string | null {
  const sections = streamSections(content);
  const preferred = ["combined", "stderr", "stdout"]
    .map((title) => sections.find((section) => section.title === title))
    .filter((section): section is ProcessOutputSection => Boolean(section));
  for (const section of preferred) {
    const line = section.content
      .split("\n")
      .map((item) => item.trim())
      .filter(Boolean)
      .at(-1);
    if (line) return line;
  }
  return null;
}

export function transcriptCursor(
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
