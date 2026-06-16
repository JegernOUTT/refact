export type TabDragKind = "chat" | "task";

export type TabDragPayload = {
  type: TabDragKind;
  id: string;
};

export function tabDragData(type: TabDragKind, id: string): string {
  return `${type}:${id}`;
}

export function parseTabDragData(value: string): TabDragPayload | null {
  const [type, ...idParts] = value.split(":");
  const id = idParts.join(":");
  if ((type === "chat" || type === "task") && id) {
    return { type, id };
  }
  return null;
}
