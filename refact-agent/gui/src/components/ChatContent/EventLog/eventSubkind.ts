import React from "react";
import type { EventSubkind } from "../../../services/refact/types";
import type { LucideIcon } from "lucide-react";
import {
  AlarmClock,
  CheckCircle2,
  Clock3,
  FileText,
  Flag,
  Info,
  Microscope,
  Monitor,
  OctagonX,
  RefreshCw,
} from "lucide-react";
import { Icon } from "../../ui";

const EVENT_SUBKIND_ICONS: Partial<Record<EventSubkind, LucideIcon>> = {
  mode_switch: RefreshCw,
  tool_decision: CheckCircle2,
  ide_callback: Monitor,
  process_completed: Flag,
  cron_fire: AlarmClock,
  tick: Clock3,
  summarization_marker: FileText,
  cancellation_note: OctagonX,
  verifier_report: Microscope,
  system_notice: Info,
};

export function eventSubkindIcon(subkind: EventSubkind): LucideIcon {
  return EVENT_SUBKIND_ICONS[subkind] ?? Info;
}

export function eventSubkindIconElement(subkind: EventSubkind): React.ReactElement {
  return React.createElement(Icon, {
    icon: eventSubkindIcon(subkind),
    size: "sm",
  });
}
