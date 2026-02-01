import type { AtCommandType, AtCommandToken, ChipDisplayInfo } from "./types";
import { formatLineRange } from "./parseAtCommands";

const ICONS: Record<AtCommandType, string> = {
  file: "📎",
  web: "🌐",
  tree: "🌲",
  search: "🔍",
  definition: "📍",
  "knowledge-load": "🧠",
  references: "📍",
  help: "❓",
};

export function getFilename(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

export function getDomain(url: string): string {
  try {
    const parsed = new URL(url.startsWith("http") ? url : `https://${url}`);
    return parsed.hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

export function getDisplayLabel(
  token: AtCommandToken,
  allTokens?: AtCommandToken[],
): string {
  const { type, arg, lineRange } = token;

  if (!arg) {
    return token.command;
  }

  switch (type) {
    case "file": {
      let filename = getFilename(arg);
      if (allTokens) {
        const sameNameTokens = allTokens.filter(
          (t) => t.type === "file" && t.arg && getFilename(t.arg) === filename,
        );
        if (sameNameTokens.length > 1) {
          const parts = arg.split(/[/\\]/);
          filename = parts.slice(-2).join("/");
        }
      }
      return lineRange ? `${filename}${formatLineRange(lineRange)}` : filename;
    }
    case "web":
      return getDomain(arg);
    case "tree":
      return arg ? getFilename(arg) : "tree";
    case "search":
    case "definition":
    case "references":
    case "knowledge-load":
      return arg.length > 20 ? arg.slice(0, 20) + "…" : arg;
    default:
      return token.command;
  }
}

export function tokenToChipInfo(
  token: AtCommandToken,
  disabled: boolean,
  allTokens?: AtCommandToken[],
): ChipDisplayInfo {
  return {
    type: token.type,
    icon: ICONS[token.type],
    label: getDisplayLabel(token, allTokens),
    fullPath: token.arg,
    lineRange: token.lineRange ? formatLineRange(token.lineRange) : undefined,
    url: token.type === "web" ? token.arg : undefined,
    disabled,
  };
}
