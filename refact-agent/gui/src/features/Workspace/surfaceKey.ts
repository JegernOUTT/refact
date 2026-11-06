import {
  decodeGitDiffDescriptor,
  encodeGitDiffDescriptor,
  type GitDiffDescriptor,
} from "./GitPanel/diff/gitDiffDescriptor";

export type WorkspaceCapabilities = Record<
  "filesPanel" | "gitPanel" | "terminalPanel",
  boolean
>;
export type MainSurfaceKind = "git";
export type PrefixedSurfaceKind =
  | "chat"
  | "task"
  | "buddy"
  | "design"
  | "file"
  | "gitdiff";
export type SurfaceKind = PrefixedSurfaceKind | "dashboard" | MainSurfaceKind;
export type SurfaceKey = string;
export type TaskSurfaceKey = `task:${string}`;
export type BuddySurfaceKey = "buddy:home";
export type PaneSurfaceKey =
  | ChatSurfaceKey
  | FileSurfaceKey
  | TaskSurfaceKey
  | BuddySurfaceKey
  | DesignSurfaceKey
  | MainSurfaceKey;
export type ChatSurfaceKey = `chat:${string}`;
export type DesignSurfaceKey = `design:${string}`;
export type FileSurfaceKey = `file:${string}`;
export type GitDiffSurfaceKey = `gitdiff:${string}`;
/**
 * Top-level, non-chat panel surfaces gated by the Git capability: the Git
 * tool surface itself and its diff editor tabs.
 */
export type MainSurfaceKey = "git:main" | GitDiffSurfaceKey;

export type ParsedSurfaceKey =
  | { kind: PrefixedSurfaceKind; id: string }
  | { kind: "git"; id: "main" }
  | { kind: "dashboard"; id: null };

const isPrefixedSurfaceKind = (kind: string): kind is PrefixedSurfaceKind =>
  kind === "chat" ||
  kind === "task" ||
  kind === "buddy" ||
  kind === "design" ||
  kind === "file" ||
  kind === "gitdiff";

export const isMainSurfaceKind = (kind: string): kind is MainSurfaceKind =>
  kind === "git";

export function makeSurfaceKey(kind: "dashboard", id?: null): SurfaceKey;
export function makeSurfaceKey(
  kind: MainSurfaceKind,
  id: "main",
): MainSurfaceKey;
export function makeSurfaceKey(
  kind: Exclude<SurfaceKind, "dashboard" | MainSurfaceKind>,
  id: string,
): SurfaceKey;
export function makeSurfaceKey(
  kind: SurfaceKind,
  id?: string | null,
): SurfaceKey {
  if (kind === "dashboard") {
    return "dashboard";
  }

  if (isMainSurfaceKind(kind)) {
    if (id !== "main") {
      throw new Error(`invalid ${kind} surface id`);
    }
    return `${kind}:main`;
  }

  if (!id) {
    throw new Error(`missing ${kind} surface id`);
  }

  return `${kind}:${id}`;
}

export function parseSurfaceKey(key: SurfaceKey): ParsedSurfaceKey {
  if (key === "dashboard") {
    return { kind: "dashboard", id: null };
  }

  const separatorIndex = key.indexOf(":");
  const kind = key.slice(0, separatorIndex);
  const id = key.slice(separatorIndex + 1);

  if (separatorIndex > 0 && id === "main" && isMainSurfaceKind(kind)) {
    return { kind, id };
  }

  if (separatorIndex <= 0 || !isPrefixedSurfaceKind(kind) || !id) {
    throw new Error(`invalid surface key: ${key}`);
  }

  if (
    (kind === "buddy" && id !== "home") ||
    (kind === "gitdiff" && !decodeGitDiffDescriptor(id))
  ) {
    throw new Error(`invalid surface key: ${key}`);
  }

  return { kind, id };
}

export const makeGitDiffSurfaceKey = (
  descriptor: GitDiffDescriptor,
): GitDiffSurfaceKey => `gitdiff:${encodeGitDiffDescriptor(descriptor)}`;

export const gitDiffDescriptorFromSurfaceKey = (
  key: SurfaceKey,
): GitDiffDescriptor | null =>
  key.startsWith("gitdiff:")
    ? decodeGitDiffDescriptor(key.slice("gitdiff:".length))
    : null;

export const isGitDiffSurface = (key: SurfaceKey): key is GitDiffSurfaceKey =>
  gitDiffDescriptorFromSurfaceKey(key) !== null;

export const isChatSurface = (key: SurfaceKey): key is ChatSurfaceKey =>
  key.startsWith("chat:") && key.length > "chat:".length;

export const isFileSurface = (key: SurfaceKey): key is FileSurfaceKey =>
  key.startsWith("file:") && key.length > "file:".length;

export const isSplittableSurface = (key: SurfaceKey): key is PaneSurfaceKey => {
  try {
    return parseSurfaceKey(key).kind !== "dashboard";
  } catch {
    return false;
  }
};

export const isDesignSurface = (key: SurfaceKey): key is DesignSurfaceKey =>
  key.startsWith("design:") && key.length > "design:".length;

export const isMainSurface = (key: SurfaceKey): key is MainSurfaceKey =>
  key === "git:main" || isGitDiffSurface(key);

export const isFilesSurface = (key: SurfaceKey): boolean =>
  key === "files:main";

export const isGitSurface = (key: SurfaceKey): key is "git:main" =>
  key === "git:main";

export const isTerminalSurface = (key: SurfaceKey): boolean =>
  key === "terminal:main";

export const isWorkspaceSurface = (key: SurfaceKey): key is PaneSurfaceKey =>
  isSplittableSurface(key);

export const isWorkspaceSurfaceEnabled = (
  key: SurfaceKey,
  capabilities: WorkspaceCapabilities,
): boolean => {
  if (!isWorkspaceSurface(key)) return false;
  if (isFileSurface(key)) return capabilities.filesPanel;
  if (isMainSurface(key)) return isMainSurfaceEnabled(key, capabilities);
  return true;
};

export const isMainSurfaceEnabled = (
  key: SurfaceKey,
  capabilities: WorkspaceCapabilities,
): key is MainSurfaceKey => {
  if (isGitDiffSurface(key)) {
    return (
      capabilities.gitPanel && gitDiffDescriptorFromSurfaceKey(key) !== null
    );
  }
  if (!isGitSurface(key)) return false;
  const { kind } = parseSurfaceKey(key);
  return kind === "git" && capabilities.gitPanel;
};
