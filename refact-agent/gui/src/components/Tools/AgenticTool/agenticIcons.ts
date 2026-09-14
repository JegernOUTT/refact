import {
  BookMarked,
  Bot,
  ClipboardList,
  FileText,
  Gauge,
  ListTree,
  MessageSquare,
  Radar,
  SlidersHorizontal,
  Target,
  type LucideIcon,
} from "lucide-react";
import type { AgenticIconKey } from "./agenticFamilies";

const ICONS: Record<AgenticIconKey, LucideIcon> = {
  spawn: Bot,
  message: MessageSquare,
  overview: ListTree,
  progress: Gauge,
  inspect: Radar,
  control: SlidersHorizontal,
  goal: Target,
  plan: ClipboardList,
  task: ClipboardList,
  doc: FileText,
  memory: BookMarked,
};

export function agenticIcon(key: AgenticIconKey): LucideIcon {
  return ICONS[key];
}
