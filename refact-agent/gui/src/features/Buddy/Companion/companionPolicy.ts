import type { AttentionRow } from "../../../services/refact/buddyCompanion";

export function visibleLiteral(value: string): string {
  return value.replace(
    // eslint-disable-next-line no-control-regex -- Make invisible command controls inspectable.
    /[\u0000-\u0008\u000b-\u001f\u007f\u200e\u200f\u202a-\u202e\u2066-\u2069]/g,
    (character) =>
      `\\u${character.charCodeAt(0).toString(16).padStart(4, "0")}`,
  );
}
export function canDecide(row: AttentionRow): boolean {
  return (
    row.kind === "tool_permission" &&
    row.actionable &&
    !!row.source_version &&
    !!row.tool_call_id &&
    (row.effect === "share_guarded_output" ||
      (row.effect === "execute_pending_tool" &&
        !!row.tool_name &&
        row.tool_arguments != null))
  );
}
export type Geometry = { x: number; y: number; width: number; height: number };
export function clampGeometry(
  value: Geometry,
  width: number,
  height: number,
): Geometry {
  const w = Math.min(Math.max(280, value.width), Math.max(1, width - 24));
  const h = Math.min(Math.max(240, value.height), Math.max(1, height - 24));
  return {
    x: Math.max(12, Math.min(value.x, width - w - 12)),
    y: Math.max(12, Math.min(value.y, height - h - 12)),
    width: w,
    height: h,
  };
}
export function sameRequest(a: AttentionRow, b: AttentionRow): boolean {
  return (
    a.id === b.id &&
    a.kind === b.kind &&
    JSON.stringify(a.question) === JSON.stringify(b.question) &&
    JSON.stringify(a.tool_arguments) === JSON.stringify(b.tool_arguments) &&
    a.literal_command === b.literal_command &&
    a.cwd === b.cwd &&
    a.chat_id === b.chat_id &&
    a.source_version === b.source_version &&
    a.tool_call_id === b.tool_call_id &&
    a.effect === b.effect
  );
}
