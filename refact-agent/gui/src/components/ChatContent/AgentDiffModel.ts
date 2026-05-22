import type { DiffChunk } from "../../services/refact/types";

type DiffMode = "unified" | "stat" | "name-only";

type DiffStats = {
  files: number;
  added: number;
  removed: number;
};

export type AgentDiffReport = {
  cardId: string;
  cardTitle: string;
  branch: string;
  base: string;
  mode: DiffMode;
  body: string;
  files: string[];
  stats: DiffStats;
  truncated: string | null;
  diffChunks: DiffChunk[];
  raw: string;
};

function extractField(content: string, label: string): string | null {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = content.match(
    new RegExp(`\\*\\*${escaped}:\\*\\*\\s*([^\\n]+)`, "u"),
  );
  const value = match?.[1]?.trim();
  return value ? value : null;
}

function extractFence(
  content: string,
): { language: string; body: string } | null {
  const match = content.match(/```([^\n`]*)\n([\s\S]*?)\n```/u);
  if (!match) return null;
  return { language: match[1].trim(), body: match[2] };
}

function parseNumstatLine(
  line: string,
): { added: number; removed: number } | null {
  const match = line.trim().match(/^(\d+)\s+(\d+)\s+\S/u);
  if (!match) return null;
  return { added: Number(match[1]), removed: Number(match[2]) };
}

function parseStatLine(
  line: string,
): { file: string; added: number; removed: number } | null {
  const separator = line.indexOf("|");
  if (separator < 0) return null;
  const file = line.slice(0, separator).trim();
  const rest = line.slice(separator + 1);
  if (!file || /files? changed/u.test(file)) return null;
  return {
    file,
    added: (rest.match(/\+/gu) ?? []).length,
    removed: (rest.match(/-/gu) ?? []).length,
  };
}

function uniqueValues(values: string[]): string[] {
  return [...new Set(values.filter(Boolean))];
}

function trimDiffPrefix(path: string): string {
  return path.replace(/^[ab]\//u, "");
}

function extractUnifiedFiles(body: string): string[] {
  const files: string[] = [];
  for (const line of body.split("\n")) {
    const match = line.match(/^diff --git a\/(.+?) b\/(.+)$/u);
    if (match) {
      files.push(match[2]);
      continue;
    }
    const fileMatch = line.match(/^\+\+\+\s+(.+)$/u);
    if (fileMatch && fileMatch[1] !== "/dev/null") {
      files.push(trimDiffPrefix(fileMatch[1]));
    }
  }
  return uniqueValues(files);
}

function extractStatFiles(body: string): string[] {
  return uniqueValues(
    body.split("\n").flatMap((line) => {
      const stat = parseStatLine(line);
      return stat ? [stat.file] : [];
    }),
  );
}

function extractNameOnlyFiles(body: string): string[] {
  return uniqueValues(
    body
      .split("\n")
      .map((line) => line.trim())
      .filter(
        (line) =>
          Boolean(line) &&
          !line.startsWith("## ") &&
          !line.startsWith("... (") &&
          line !== "(no changes detected)" &&
          line !== "(no changes)",
      ),
  );
}

function detectMode(language: string, body: string): DiffMode {
  if (language === "diff" || body.includes("diff --git")) return "unified";
  if (body.split("\n").some((line) => line.includes("|"))) return "stat";
  return "name-only";
}

function computeStats(
  body: string,
  files: string[],
  mode: DiffMode,
): DiffStats {
  if (mode === "unified") {
    let added = 0;
    let removed = 0;
    for (const line of body.split("\n")) {
      if (line.startsWith("+") && !line.startsWith("+++")) added += 1;
      if (line.startsWith("-") && !line.startsWith("---")) removed += 1;
    }
    return { files: files.length, added, removed };
  }

  if (mode === "stat") {
    let added = 0;
    let removed = 0;
    let statFiles = 0;
    for (const line of body.split("\n")) {
      const numstat = parseNumstatLine(line);
      if (numstat) {
        added += numstat.added;
        removed += numstat.removed;
        statFiles += 1;
        continue;
      }
      const stat = parseStatLine(line);
      if (stat) {
        added += stat.added;
        removed += stat.removed;
        statFiles += 1;
      }
    }
    return { files: files.length || statFiles, added, removed };
  }

  return { files: files.length, added: 0, removed: 0 };
}

function extractFiles(body: string, mode: DiffMode): string[] {
  switch (mode) {
    case "unified":
      return extractUnifiedFiles(body);
    case "stat":
      return extractStatFiles(body);
    case "name-only":
      return extractNameOnlyFiles(body);
  }
}

function extractTruncation(body: string): string | null {
  return (
    body
      .split("\n")
      .map((line) => line.trim())
      .find(
        (line) => line.startsWith("... (") && line.includes("more lines"),
      ) ?? null
  );
}

function hunkHeaderStart(line: string): number {
  const match = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/u);
  return match ? Number(match[1]) : 1;
}

function unifiedDiffToChunks(body: string): DiffChunk[] {
  const chunks: DiffChunk[] = [];
  let currentFile: string | null = null;
  let line1 = 1;
  let removed: string[] = [];
  let added: string[] = [];

  const flush = () => {
    if (!currentFile || (removed.length === 0 && added.length === 0)) return;
    chunks.push({
      file_name: currentFile,
      file_action: "edit",
      line1,
      line2: line1,
      lines_remove: removed.join("\n"),
      lines_add: added.join("\n"),
    });
    removed = [];
    added = [];
  };

  for (const line of body.split("\n")) {
    const fileMatch = line.match(/^diff --git a\/(.+?) b\/(.+)$/u);
    if (fileMatch) {
      flush();
      currentFile = fileMatch[2];
      line1 = 1;
      continue;
    }

    if (line.startsWith("@@")) {
      flush();
      line1 = hunkHeaderStart(line);
      continue;
    }

    if (line.startsWith("---") || line.startsWith("+++")) continue;
    if (line.startsWith("+") && currentFile) {
      added.push(line.slice(1));
    } else if (line.startsWith("-") && currentFile) {
      removed.push(line.slice(1));
    }
  }

  flush();
  return chunks;
}

export function parseAgentDiffOutput(content: string): AgentDiffReport | null {
  const titleMatch = content.match(/^# Agent Diff for\s+(\S+)/mu);
  if (!titleMatch) return null;

  const fence = extractFence(content);
  if (!fence) return null;

  const mode = detectMode(fence.language, fence.body);
  const files = extractFiles(fence.body, mode);
  const stats = computeStats(fence.body, files, mode);

  return {
    cardId: titleMatch[1],
    cardTitle: extractField(content, "Card") ?? titleMatch[1],
    branch: extractField(content, "Branch") ?? "unknown",
    base: extractField(content, "Base") ?? "unknown",
    mode,
    body: fence.body,
    files,
    stats,
    truncated: extractTruncation(fence.body),
    diffChunks: mode === "unified" ? unifiedDiffToChunks(fence.body) : [],
    raw: content,
  };
}
