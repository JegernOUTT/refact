import type {
  AtCommandType,
  AtCommandToken,
  Token,
  LineRange,
  ParsedLine,
} from "./types";

const AT_COMMANDS: AtCommandType[] = [
  "file",
  "web",
  "tree",
  "search",
  "definition",
  "knowledge-load",
  "references",
  "help",
];

const TRAILING_PUNCTUATION = /[!.,?]+$/;

export function parseLineRange(arg: string): {
  path: string;
  lineRange?: LineRange;
} {
  const rangeMatch = arg.match(/:(\d+)?-(\d+)?$/);
  if (rangeMatch) {
    const path = arg.replace(/:(\d+)?-(\d+)?$/, "");
    const line1 = rangeMatch[1] ? parseInt(rangeMatch[1], 10) : undefined;
    const line2 = rangeMatch[2] ? parseInt(rangeMatch[2], 10) : undefined;

    if (line1 !== undefined && line2 !== undefined) {
      return { path, lineRange: { line1, line2, kind: "range" } };
    } else if (line1 !== undefined) {
      return { path, lineRange: { line1, kind: "to-end" } };
    } else if (line2 !== undefined) {
      return { path, lineRange: { line1: 1, line2, kind: "from-start" } };
    }
  }

  const singleMatch = arg.match(/:(\d+)$/);
  if (singleMatch) {
    const path = arg.replace(/:(\d+)$/, "");
    return {
      path,
      lineRange: { line1: parseInt(singleMatch[1], 10), kind: "single" },
    };
  }

  return { path: arg };
}

export function formatLineRange(lineRange: LineRange): string {
  switch (lineRange.kind) {
    case "single":
      return `:${lineRange.line1}`;
    case "range":
      return `:${lineRange.line1}-${lineRange.line2}`;
    case "to-end":
      return `:${lineRange.line1}-`;
    case "from-start":
      return `:-${lineRange.line2}`;
  }
}

function isAtCommand(word: string): AtCommandType | null {
  if (!word.startsWith("@")) return null;
  const cmd = word.slice(1).toLowerCase();
  return AT_COMMANDS.find((c) => c === cmd) ?? null;
}

function parseWords(line: string): [string, number, number][] {
  const results: [string, number, number][] = [];
  const regex = /@?\S+/g;
  let match;

  while ((match = regex.exec(line)) !== null) {
    const trimmed = match[0].replace(TRAILING_PUNCTUATION, "");
    if (trimmed.length > 0) {
      results.push([trimmed, match.index, match.index + trimmed.length]);
    }
  }

  return results;
}

export function parseLine(line: string): ParsedLine {
  const tokens: Token[] = [];
  const words = parseWords(line);

  let lastEnd = 0;
  let i = 0;

  while (i < words.length) {
    const [word, startIdx, endIdx] = words[i];
    const cmdType = isAtCommand(word);

    if (cmdType) {
      if (startIdx > lastEnd) {
        tokens.push({
          kind: "text",
          text: line.slice(lastEnd, startIdx),
          startIndex: lastEnd,
          endIndex: startIdx,
        });
      }

      const args: string[] = [];
      let argEndIdx = endIdx;
      let j = i + 1;

      while (j < words.length) {
        const [nextWord, , nextEnd] = words[j];
        if (isAtCommand(nextWord)) break;
        args.push(nextWord);
        argEndIdx = nextEnd;
        j++;
      }

      const rawText = line.slice(startIdx, argEndIdx);
      const arg = args.length > 0 ? args.join(" ") : undefined;

      const token: AtCommandToken = {
        kind: "at",
        type: cmdType,
        raw: rawText,
        command: word,
        arg,
        startIndex: startIdx,
        endIndex: argEndIdx,
      };

      if (cmdType === "file" && arg) {
        const { path, lineRange } = parseLineRange(arg);
        token.arg = path;
        token.lineRange = lineRange;
      }

      tokens.push(token);
      lastEnd = argEndIdx;
      i = j;
    } else {
      i++;
    }
  }

  if (lastEnd < line.length) {
    tokens.push({
      kind: "text",
      text: line.slice(lastEnd),
      startIndex: lastEnd,
      endIndex: line.length,
    });
  }

  if (tokens.length === 0 && line.length > 0) {
    tokens.push({
      kind: "text",
      text: line,
      startIndex: 0,
      endIndex: line.length,
    });
  }

  return { tokens, originalText: line };
}

export function parseLines(text: string): ParsedLine[] {
  const lines = text.split("\n");
  const results: ParsedLine[] = [];
  let inCodeFence = false;

  for (const line of lines) {
    if (line.trimStart().startsWith("```")) {
      inCodeFence = !inCodeFence;
      results.push({
        tokens: [
          { kind: "text", text: line, startIndex: 0, endIndex: line.length },
        ],
        originalText: line,
      });
      continue;
    }

    if (inCodeFence) {
      results.push({
        tokens: [
          { kind: "text", text: line, startIndex: 0, endIndex: line.length },
        ],
        originalText: line,
      });
    } else {
      results.push(parseLine(line));
    }
  }

  return results;
}

export function hasAtCommands(parsedLines: ParsedLine[]): boolean {
  return parsedLines.some((line) =>
    line.tokens.some((token) => token.kind === "at"),
  );
}

export function getAtCommandTokens(
  parsedLines: ParsedLine[],
): AtCommandToken[] {
  return parsedLines.flatMap((line) =>
    line.tokens.filter((token): token is AtCommandToken => token.kind === "at"),
  );
}
