export type SurfaceKind = "chat" | "task" | "buddy" | "dashboard";
export type SurfaceKey = string;
export type ChatSurfaceKey = `chat:${string}`;

export type ParsedSurfaceKey =
  | { kind: "chat" | "task" | "buddy"; id: string }
  | { kind: "dashboard"; id: null };

const isPrefixedSurfaceKind = (
  kind: string,
): kind is "chat" | "task" | "buddy" =>
  kind === "chat" || kind === "task" || kind === "buddy";

export function makeSurfaceKey(kind: "dashboard", id?: null): SurfaceKey;
export function makeSurfaceKey(
  kind: Exclude<SurfaceKind, "dashboard">,
  id: string,
): SurfaceKey;
export function makeSurfaceKey(
  kind: SurfaceKind,
  id?: string | null,
): SurfaceKey {
  if (kind === "dashboard") {
    return "dashboard";
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

  if (separatorIndex <= 0 || !isPrefixedSurfaceKind(kind) || !id) {
    throw new Error(`invalid surface key: ${key}`);
  }

  return { kind, id };
}

export const isChatSurface = (key: SurfaceKey): key is ChatSurfaceKey =>
  key.startsWith("chat:") && key.length > "chat:".length;
