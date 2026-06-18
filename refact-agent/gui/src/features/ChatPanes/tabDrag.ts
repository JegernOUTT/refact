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

const surfaceTabDragMimeType = tabDragMimeTypes.surface;

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
  const surfaceKey = dataTransfer.getData(surfaceTabDragMimeType) || undefined;
  const textPayload = parseTabDragData(dataTransfer.getData("text/plain"));
  if (textPayload) return { ...textPayload, surfaceKey };

  for (const type of Object.values(tabDragMimeTypes)) {
    const payload = parseTabDragData(dataTransfer.getData(type));
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
  const payload = readTabDragData(dataTransfer);
  if (payload) return payload.type === type;
  return Array.from(dataTransfer.types).includes(tabDragMimeTypes[type]);
}
