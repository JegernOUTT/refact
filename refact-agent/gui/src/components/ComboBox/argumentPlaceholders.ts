export type PlaceholderRange = { start: number; end: number };

const PLACEHOLDER_PATTERN = "<[^<>\\n]+>|\\[[^\\[\\]\\n]+\\]";

function makePlaceholderRegExp(): RegExp {
  return new RegExp(PLACEHOLDER_PATTERN, "g");
}

export function findPlaceholderRanges(text: string): PlaceholderRange[] {
  const re = makePlaceholderRegExp();
  const ranges: PlaceholderRange[] = [];
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    ranges.push({ start: match.index, end: match.index + match[0].length });
    if (match.index === re.lastIndex) re.lastIndex += 1;
  }
  return ranges;
}

export function parseHintPlaceholders(hint: string): string[] {
  const re = makePlaceholderRegExp();
  const tokens: string[] = [];
  let match: RegExpExecArray | null;
  while ((match = re.exec(hint)) !== null) {
    tokens.push(match[0]);
  }
  return tokens;
}

export function nextPlaceholder(
  text: string,
  from: number,
): PlaceholderRange | null {
  for (const range of findPlaceholderRanges(text)) {
    if (range.start >= from) return range;
  }
  return null;
}

export function previousPlaceholder(
  text: string,
  before: number,
): PlaceholderRange | null {
  let previous: PlaceholderRange | null = null;
  for (const range of findPlaceholderRanges(text)) {
    if (range.end <= before) previous = range;
    else break;
  }
  return previous;
}

export function placeholderAt(
  text: string,
  position: number,
): PlaceholderRange | null {
  for (const range of findPlaceholderRanges(text)) {
    if (position >= range.start && position <= range.end) return range;
  }
  return null;
}

export function selectionIsPlaceholder(
  text: string,
  selectionStart: number,
  selectionEnd: number,
): boolean {
  if (selectionStart === selectionEnd) return false;
  return findPlaceholderRanges(text).some(
    (range) => range.start === selectionStart && range.end === selectionEnd,
  );
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function stripUnfilledPlaceholders(
  text: string,
  tokens: string[],
): string {
  let out = text;
  for (const token of tokens) {
    if (!token) continue;
    const escaped = escapeRegExp(token);
    out = out.replace(
      new RegExp(`( ?)${escaped}( ?)`, "g"),
      (_match, lead: string, trail: string) => (lead ? trail : ""),
    );
  }
  return out.replace(/[ \t]+(?=\n|$)/g, "");
}
