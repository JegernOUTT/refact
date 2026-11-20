export function isLinkedTaskChat(
  mode: string | null | undefined,
  role?: string | null,
): boolean {
  if (role) return role !== "planner";
  return Boolean(mode) && mode?.toLowerCase() !== "task_planner";
}

export function taskChatKindLabel(
  mode: string | null | undefined,
  role?: string | null,
): "Linked chat" | "Planner" {
  return isLinkedTaskChat(mode, role) ? "Linked chat" : "Planner";
}
