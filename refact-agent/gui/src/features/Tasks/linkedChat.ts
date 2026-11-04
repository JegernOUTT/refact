export function isLinkedTaskChat(
  mode: string | null | undefined,
  role?: string | null,
): boolean {
  if (role === "linked") return true;
  return Boolean(mode) && mode?.toLowerCase() !== "task_planner";
}

export function taskChatKindLabel(
  mode: string | null | undefined,
  role?: string | null,
): "Linked chat" | "Planner" {
  return isLinkedTaskChat(mode, role) ? "Linked chat" : "Planner";
}
