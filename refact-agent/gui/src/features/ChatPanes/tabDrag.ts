import { makeSurfaceKey, type SurfaceKey } from "../Workspace/surfaceKey";

export type TabDragKind = "chat" | "task" | "buddy" | "surface";

export type TabDragPayload = {
  type: TabDragKind;
  id: string;
  surfaceKey?: SurfaceKey;
};

const tabDragMimeTypes: Record<TabDragKind, string> = {
  chat: "application/x-refact-chat-tab",
  task: "application/x-refact-task-tab",
  buddy: "application/x-refact-buddy-tab",
  surface: "application/x-refact-surface-tab",
};

const knownTabDragMimeTypes = Object.values(tabDragMimeTypes);
const surfaceTabDragMimeType = tabDragMimeTypes.surface;

function dataTransferTypes(dataTransfer: DataTransfer): string[] {
  return Array.from(dataTransfer.types);
}

function hasKnownTabDragMimeType(dataTransfer: DataTransfer): boolean {
  const types = dataTransferTypes(dataTransfer);
  return knownTabDragMimeTypes.some((mimeType) => types.includes(mimeType));
}

export function tabDragData(type: TabDragKind, id: string): string {
  return `${type}:${id}`;
}

export function setTabDragData(
  dataTransfer: DataTransfer,
  type: TabDragKind,
  id: string,
  surfaceKey?: SurfaceKey,
): void {
  const value = tabDragData(type, id);
  dataTransfer.setData("text/plain", value);
  dataTransfer.setData(tabDragMimeTypes[type], value);
  if (surfaceKey) {
    dataTransfer.setData(surfaceTabDragMimeType, surfaceKey);
  }
}

export function parseTabDragData(value: string): TabDragPayload | null {
  const [type, ...idParts] = value.split(":");
  const id = idParts.join(":");
  if (
    (type === "chat" ||
      type === "task" ||
      type === "buddy" ||
      type === "surface") &&
    id
  ) {
    return { type, id };
  }
  return null;
}

export function readTabDragData(
  dataTransfer: DataTransfer,
): TabDragPayload | null {
  if (!hasKnownTabDragMimeType(dataTransfer)) return null;

  const surfaceKey = dataTransfer.getData(surfaceTabDragMimeType) || undefined;
  const textPayload = parseTabDragData(dataTransfer.getData("text/plain"));
  if (textPayload) return { ...textPayload, surfaceKey };

  for (const mimeType of knownTabDragMimeTypes) {
    const payload = parseTabDragData(dataTransfer.getData(mimeType));
    if (payload) return { ...payload, surfaceKey };
  }

  return null;
}

export function surfaceKeyFromTabDragPayload(
  payload: TabDragPayload | null,
): SurfaceKey | null {
  if (!payload) return null;
  if (payload.surfaceKey) return payload.surfaceKey;
  if (payload.type === "surface") return payload.id;
  return makeSurfaceKey(payload.type, payload.id);
}

export function readTabDragSurfaceKey(
  dataTransfer: DataTransfer,
): SurfaceKey | null {
  return surfaceKeyFromTabDragPayload(readTabDragData(dataTransfer));
}

export function hasTabDragType(
  dataTransfer: DataTransfer,
  type: TabDragKind,
): boolean {
  const types = dataTransferTypes(dataTransfer);
  if (!types.some((mimeType) => knownTabDragMimeTypes.includes(mimeType))) {
    return false;
  }
  if (types.includes(tabDragMimeTypes[type])) return true;

  const payload = readTabDragData(dataTransfer);
  if (payload) return payload.type === type;
  return false;
}
