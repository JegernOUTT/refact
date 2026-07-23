export const PANEL_KINDS = ["files", "git", "terminal"] as const;
export const CENTER_PANEL_KINDS = ["git"] as const;

export type PanelKind = (typeof PANEL_KINDS)[number];
export type CenterPanelKind = (typeof CENTER_PANEL_KINDS)[number];
export type PanelCapabilityKey = `${PanelKind}Panel`;
export type PanelCapabilities = Record<PanelCapabilityKey, boolean>;
export type SurfaceKind =
  | "chat"
  | "task"
  | "buddy"
  | "dashboard"
  | "file"
  | PanelKind;
export type SurfaceKey = string;
export type ChatSurfaceKey = `chat:${string}`;
export type FileSurfaceKey = `file:${string}`;
export type PanelSurfaceKey = `${PanelKind}:main`;

export type ParsedSurfaceKey =
  | { kind: "chat" | "task" | "buddy" | "file"; id: string }
  | { kind: PanelKind; id: "main" }
  | { kind: "dashboard"; id: null };

const isPrefixedSurfaceKind = (
  kind: string,
): kind is "chat" | "task" | "buddy" | "file" =>
  kind === "chat" || kind === "task" || kind === "buddy" || kind === "file";

export const isPanelKind = (kind: string): kind is PanelKind =>
  (PANEL_KINDS as readonly string[]).includes(kind);

export const isCenterPanelKind = (kind: string): kind is CenterPanelKind =>
  (CENTER_PANEL_KINDS as readonly string[]).includes(kind);

export function makeSurfaceKey(kind: "dashboard", id?: null): SurfaceKey;
export function makeSurfaceKey(kind: PanelKind, id: "main"): PanelSurfaceKey;
export function makeSurfaceKey(
  kind: Exclude<SurfaceKind, "dashboard" | PanelKind>,
  id: string,
): SurfaceKey;
export function makeSurfaceKey(
  kind: SurfaceKind,
  id?: string | null,
): SurfaceKey {
  if (kind === "dashboard") {
    return "dashboard";
  }

  if (isPanelKind(kind)) {
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

  if (separatorIndex > 0 && isPanelKind(kind) && id === "main") {
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

export const isPanelSurface = (key: SurfaceKey): key is PanelSurfaceKey =>
  PANEL_KINDS.some((kind) => key === `${kind}:main`);

export const isCenterPanelSurface = (
  key: SurfaceKey,
): key is `${CenterPanelKind}:main` =>
  CENTER_PANEL_KINDS.some((kind) => key === `${kind}:main`);

export const isFilesSurface = (
  key: SurfaceKey,
): key is Extract<PanelSurfaceKey, "files:main"> => key === "files:main";

export const isGitSurface = (
  key: SurfaceKey,
): key is Extract<PanelSurfaceKey, "git:main"> => key === "git:main";

export const isTerminalSurface = (
  key: SurfaceKey,
): key is Extract<PanelSurfaceKey, "terminal:main"> => key === "terminal:main";

export const isWorkspaceSurface = (
  key: SurfaceKey,
): key is ChatSurfaceKey | FileSurfaceKey | PanelSurfaceKey =>
  isChatSurface(key) || isFileSurface(key) || isCenterPanelSurface(key);

export const isWorkspaceSurfaceEnabled = (
  key: SurfaceKey,
  capabilities: PanelCapabilities,
): boolean =>
  isChatSurface(key) ||
  (isFileSurface(key) && capabilities.filesPanel) ||
  isPanelSurfaceEnabled(key, capabilities);

export const panelCapabilityKey = (kind: PanelKind): PanelCapabilityKey =>
  `${kind}Panel`;

export const isPanelSurfaceEnabled = (
  key: SurfaceKey,
  capabilities: PanelCapabilities,
): key is PanelSurfaceKey => {
  if (!isCenterPanelSurface(key)) return false;
  const { kind } = parseSurfaceKey(key);
  return isCenterPanelKind(kind) && capabilities[panelCapabilityKey(kind)];
};
