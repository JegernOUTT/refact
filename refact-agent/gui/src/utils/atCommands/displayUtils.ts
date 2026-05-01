import type { AtCommandType, AtCommandToken, ChipDisplayInfo } from "./types";
import { formatLineRange } from "./parseAtCommands";

type CommandMeta = {
  argRequired: boolean;
  clickable: boolean;
};

const COMMAND_META: Record<AtCommandType, CommandMeta> = {
  file: { argRequired: true, clickable: true },
  web: { argRequired: true, clickable: true },
  tree: { argRequired: false, clickable: false },
  search: { argRequired: true, clickable: false },
  definition: { argRequired: true, clickable: false },
  "knowledge-load": { argRequired: true, clickable: false },
  references: { argRequired: true, clickable: false },
  help: { argRequired: false, clickable: false },
};

export function isCommandDisabled(
  token: AtCommandToken,
  hostDisabled: boolean,
): boolean {
  const meta = COMMAND_META[token.type];
  if (hostDisabled) return true;
  if (meta.argRequired && !token.arg) return true;
  return false;
}

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
  hostDisabled: boolean,
  allTokens?: AtCommandToken[],
): ChipDisplayInfo {
  return {
    type: token.type,
    label: getDisplayLabel(token, allTokens),
    fullPath: token.arg,
    lineRange: token.lineRange ? formatLineRange(token.lineRange) : undefined,
    url: token.type === "web" ? token.arg : undefined,
    disabled: isCommandDisabled(token, hostDisabled),
  };
}
