export type WorkspaceCapabilities = Record<
  "filesPanel" | "gitPanel" | "terminalPanel",
  boolean
>;
export type MainSurfaceKind = "git";
export type SurfaceKind =
  | "chat"
  | "task"
  | "buddy"
  | "dashboard"
  | "design"
  | "file"
  | MainSurfaceKind;
export type SurfaceKey = string;
export type ChatSurfaceKey = `chat:${string}`;
export type DesignSurfaceKey = `design:${string}`;
export type FileSurfaceKey = `file:${string}`;
export type MainSurfaceKey = "git:main";

export type ParsedSurfaceKey =
  | { kind: "chat" | "task" | "buddy" | "design" | "file"; id: string }
  | { kind: "git"; id: "main" }
  | { kind: "dashboard"; id: null };

const isPrefixedSurfaceKind = (
  kind: string,
): kind is "chat" | "task" | "buddy" | "design" | "file" =>
  kind === "chat" ||
  kind === "task" ||
  kind === "buddy" ||
  kind === "design" ||
  kind === "file";

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

  return { kind, id };
}

export const isChatSurface = (key: SurfaceKey): key is ChatSurfaceKey =>
  key.startsWith("chat:") && key.length > "chat:".length;

export const isFileSurface = (key: SurfaceKey): key is FileSurfaceKey =>
  key.startsWith("file:") && key.length > "file:".length;

export const isDesignSurface = (key: SurfaceKey): key is DesignSurfaceKey =>
  key.startsWith("design:") && key.length > "design:".length;

export const isMainSurface = (key: SurfaceKey): key is MainSurfaceKey =>
  key === "git:main";

export const isFilesSurface = (key: SurfaceKey): boolean =>
  key === "files:main";

export const isGitSurface = (key: SurfaceKey): key is MainSurfaceKey =>
  key === "git:main";

export const isTerminalSurface = (key: SurfaceKey): boolean =>
  key === "terminal:main";

export const isWorkspaceSurface = (
  key: SurfaceKey,
): key is ChatSurfaceKey | DesignSurfaceKey | FileSurfaceKey | MainSurfaceKey =>
  isChatSurface(key) ||
  isDesignSurface(key) ||
  isFileSurface(key) ||
  isGitSurface(key);

export const isWorkspaceSurfaceEnabled = (
  key: SurfaceKey,
  capabilities: WorkspaceCapabilities,
): boolean =>
  isChatSurface(key) ||
  isDesignSurface(key) ||
  (isFileSurface(key) && capabilities.filesPanel) ||
  isMainSurfaceEnabled(key, capabilities);

export const isMainSurfaceEnabled = (
  key: SurfaceKey,
  capabilities: WorkspaceCapabilities,
): key is MainSurfaceKey => {
  if (!isGitSurface(key)) return false;
  const { kind } = parseSurfaceKey(key);
  return kind === "git" && capabilities.gitPanel;
};
