export type TabDragKind = "chat" | "task";

export type TabDragPayload = {
  type: TabDragKind;
  id: string;
};

const tabDragMimeTypes: Record<TabDragKind, string> = {
  chat: "application/x-refact-chat-tab",
  task: "application/x-refact-task-tab",
};

export function tabDragData(type: TabDragKind, id: string): string {
  return `${type}:${id}`;
}

export function setTabDragData(
  dataTransfer: DataTransfer,
  type: TabDragKind,
  id: string,
): void {
  const value = tabDragData(type, id);
  dataTransfer.setData("text/plain", value);
  dataTransfer.setData(tabDragMimeTypes[type], value);
}

export function parseTabDragData(value: string): TabDragPayload | null {
  const [type, ...idParts] = value.split(":");
  const id = idParts.join(":");
  if ((type === "chat" || type === "task") && id) {
    return { type, id };
  }
  return null;
}

export function readTabDragData(
  dataTransfer: DataTransfer,
): TabDragPayload | null {
  const textPayload = parseTabDragData(dataTransfer.getData("text/plain"));
  if (textPayload) return textPayload;

  for (const type of Object.values(tabDragMimeTypes)) {
    const payload = parseTabDragData(dataTransfer.getData(type));
    if (payload) return payload;
  }

  return null;
}

export function hasTabDragType(
  dataTransfer: DataTransfer,
  type: TabDragKind,
): boolean {
  const payload = readTabDragData(dataTransfer);
  if (payload) return payload.type === type;
  return Array.from(dataTransfer.types).includes(tabDragMimeTypes[type]);
}
