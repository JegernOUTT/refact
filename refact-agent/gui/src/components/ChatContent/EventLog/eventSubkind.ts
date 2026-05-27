import type { EventSubkind } from "../../../services/refact/types";

const EVENT_SUBKIND_ICONS: Record<EventSubkind, string> = {
  mode_switch: "🔁",
  tool_decision: "✅",
  ide_callback: "💻",
  process_completed: "🏁",
  cron_fire: "⏰",
  tick: "🕒",
  summarization_marker: "📝",
  cancellation_note: "🛑",
  verifier_report: "🔬",
  system_notice: "ℹ️",
};

export function eventSubkindIcon(subkind: EventSubkind): string {
  return EVENT_SUBKIND_ICONS[subkind];
}
